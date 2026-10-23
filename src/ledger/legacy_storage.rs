//! The storage names a database had before the rename to Loomtracer (hivemind-x4xl7): the
//! `hm_*` tables of the token, tenant and graph stores became `lt_*`.
//!
//! A table is data, so the rename is a migration and not a find-and-replace: starting a renamed
//! binary on an old database would `CREATE TABLE IF NOT EXISTS lt_*` beside the old tables, and
//! every tenant, user and token would be gone from the new binary's point of view. So the
//! schema setup of each store refuses to run while an old table exists
//! ([`legacy_storage_refusal`]), and `loomtracer migrate-storage` is the one step that moves the
//! tables across ([`rename_sqlite_storage`], and `PostgresEventLedger::rename_legacy_storage`).
//! The step runs in one transaction, checks that every table holds the same number of rows
//! before and after, and does nothing on a database that is already current.

use rusqlite::Connection;
use serde::Serialize;

use crate::error::LedgerError;
use crate::Result;

use super::backend_error::storage_error;

/// Old name and new name of every table the SQLite ledger file held, in the order they can be
/// renamed (a table's foreign keys point at the ones before it).
pub(crate) const SQLITE_TABLE_RENAMES: &[(&str, &str)] =
    &[("hm_users", "lt_users"), ("hm_tokens", "lt_tokens")];

/// The same for a Postgres database.
#[cfg_attr(not(feature = "shared-backend-postgres"), allow(dead_code))]
pub(crate) const POSTGRES_TABLE_RENAMES: &[(&str, &str)] = &[
    ("hm_tenants", "lt_tenants"),
    ("hm_users", "lt_users"),
    ("hm_tokens", "lt_tokens"),
    ("hm_oidc_users", "lt_oidc_users"),
    ("hm_nodes", "lt_nodes"),
    ("hm_edges", "lt_edges"),
];

/// Old and new name of the one SQLite index that carried the old prefix.
const SQLITE_INDEX_RENAME: (&str, &str, &str) = (
    "hm_tokens_hash_idx",
    "lt_tokens_hash_idx",
    "CREATE INDEX IF NOT EXISTS lt_tokens_hash_idx ON lt_tokens (token_hash)",
);

/// One table that moved to its new name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableRename {
    pub from: String,
    pub to: String,
    pub rows_before: u64,
    pub rows_after: u64,
}

/// What `loomtracer migrate-storage` did, or with `dry_run` what it would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageRenameReport {
    pub backend: &'static str,
    pub dry_run: bool,
    /// Tables that still had their old name and were (or would be) renamed.
    pub renamed: Vec<TableRename>,
    /// Tables that already had their new name before this ran.
    pub already_current: Vec<String>,
}

impl StorageRenameReport {
    pub(crate) fn new(backend: &'static str, dry_run: bool) -> Self {
        Self {
            backend,
            dry_run,
            renamed: Vec::new(),
            already_current: Vec::new(),
        }
    }
}

/// The error a store's schema setup raises while `legacy` old-named tables exist.
pub(crate) fn legacy_storage_refusal(legacy: &[&str]) -> LedgerError {
    LedgerError::Storage(format!(
        "this database still has the storage names from before the rename to Loomtracer ({}). \
         Starting on it would create empty lt_* tables beside them and lose every tenant, user \
         and token. Run `loomtracer migrate-storage` once against this database (add --dry-run \
         to rehearse it), then start again",
        legacy.join(", ")
    ))
}

/// The error when a table exists under both names: neither can be taken for the real one.
pub(crate) fn both_names_exist(from: &str, to: &str) -> LedgerError {
    LedgerError::Storage(format!(
        "both {from} and {to} exist, so neither can be taken for the real one and nothing was \
         renamed. Keep the one that holds the rows and drop the other, then run the step again"
    ))
}

/// The error when a table's row count differs after the rename: the rename is rolled back.
pub(crate) fn row_count_changed(table: &TableRename) -> LedgerError {
    LedgerError::Storage(format!(
        "{} held {} rows and {} holds {} after the rename: nothing was changed",
        table.from, table.rows_before, table.to, table.rows_after
    ))
}

fn sqlite_table_exists(connection: &Connection, name: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |row| row.get(0),
        )
        .map_err(|error| storage_error(error).into())
}

fn sqlite_row_count(connection: &Connection, table: &str) -> Result<u64> {
    // `table` is one of the constant names above, never input.
    let count: i64 = connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(storage_error)?;
    Ok(u64::try_from(count).unwrap_or(0))
}

/// How many rows of the token tables point at a row that is not there. A rename that left the
/// foreign key of `lt_tokens` pointing at the old name of `lt_users` fails this check with a
/// SQL error; a row that was already dangling before the rename is not the rename's doing, so
/// the caller compares the count before and after.
fn sqlite_dangling_rows(connection: &Connection) -> Result<i64> {
    connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|error| storage_error(error).into())
}

/// The old-named tables a SQLite ledger file still holds.
pub(crate) fn legacy_sqlite_tables(connection: &Connection) -> Result<Vec<&'static str>> {
    let mut legacy = Vec::new();
    for (from, _) in SQLITE_TABLE_RENAMES {
        if sqlite_table_exists(connection, from)? {
            legacy.push(*from);
        }
    }
    Ok(legacy)
}

/// Errors while the SQLite ledger file still holds an old-named table.
pub(crate) fn require_current_sqlite_names(connection: &Connection) -> Result<()> {
    let legacy = legacy_sqlite_tables(connection)?;
    if legacy.is_empty() {
        Ok(())
    } else {
        Err(legacy_storage_refusal(&legacy).into())
    }
}

/// Renames the old-named tables of a SQLite ledger file to their new names, in one transaction.
/// With `dry_run` the same work runs and is rolled back, so the report is exactly what a real
/// run would produce. Errors, changing nothing, when a table exists under both names or a
/// table's row count differs afterwards.
pub(crate) fn rename_sqlite_storage(
    connection: &mut Connection,
    dry_run: bool,
) -> Result<StorageRenameReport> {
    let mut report = StorageRenameReport::new("sqlite", dry_run);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let dangling_before = sqlite_dangling_rows(&transaction)?;

    for (from, to) in SQLITE_TABLE_RENAMES {
        let has_old = sqlite_table_exists(&transaction, from)?;
        let has_new = sqlite_table_exists(&transaction, to)?;
        match (has_old, has_new) {
            (true, true) => return Err(both_names_exist(from, to).into()),
            (false, true) => report.already_current.push((*to).to_owned()),
            (false, false) => {}
            (true, false) => {
                let rows_before = sqlite_row_count(&transaction, from)?;
                transaction
                    .execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
                    .map_err(storage_error)?;
                let rows_after = sqlite_row_count(&transaction, to)?;
                let table = TableRename {
                    from: (*from).to_owned(),
                    to: (*to).to_owned(),
                    rows_before,
                    rows_after,
                };
                if rows_before != rows_after {
                    return Err(row_count_changed(&table).into());
                }
                report.renamed.push(table);
            }
        }
    }

    // An index has no rename in SQLite: the old one goes and the new one is built on the
    // renamed table (a table that was never there needs neither).
    if sqlite_table_exists(&transaction, "lt_tokens")? {
        let (old_index, _, create_new) = SQLITE_INDEX_RENAME;
        transaction
            .execute_batch(&format!("DROP INDEX IF EXISTS {old_index}; {create_new};"))
            .map_err(storage_error)?;
    }

    // The foreign key of `lt_tokens` must follow `lt_users` to its new name.
    let dangling_after = sqlite_dangling_rows(&transaction)?;
    if dangling_after > dangling_before {
        return Err(LedgerError::Storage(format!(
            "{dangling_after} rows point at a missing row after the rename, where \
             {dangling_before} did before: nothing was changed"
        ))
        .into());
    }

    if dry_run {
        transaction.rollback().map_err(storage_error)?;
    } else {
        transaction.commit().map_err(storage_error)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
