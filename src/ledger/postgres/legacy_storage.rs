//! Postgres half of the storage rename (hivemind-x4xl7). The reasoning, and the SQLite half, are
//! in `crate::ledger::legacy_storage`.
//!
//! One transaction, taken under the same advisory lock as every schema setup, so no store sets up
//! its tables while the old ones are half renamed. Postgres renames a table with its rows,
//! indexes and the foreign keys that point at it; the constraint and index names derived from
//! the old table name (`hm_tokens_pkey`) are renamed here as well, so a migrated database and a
//! fresh one have the same catalog.

use postgres::GenericClient;

use crate::ledger::backend_error::storage_error;
use crate::ledger::legacy_storage::{
    both_names_exist, legacy_storage_refusal, row_count_changed, StorageRenameReport, TableRename,
    POSTGRES_TABLE_RENAMES,
};
use crate::Result;

use super::PostgresEventLedger;

const OLD_PREFIX: &str = "hm_";
const NEW_PREFIX: &str = "lt_";

fn table_exists(client: &mut impl GenericClient, name: &str) -> Result<bool> {
    let row = client
        .query_one("SELECT to_regclass($1::text) IS NOT NULL", &[&name])
        .map_err(storage_error)?;
    Ok(row.get(0))
}

fn row_count(client: &mut impl GenericClient, table: &str) -> Result<u64> {
    // `table` is one of the constant names, never input.
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let row = client.query_one(sql.as_str(), &[]).map_err(storage_error)?;
    let count: i64 = row.get(0);
    Ok(u64::try_from(count).unwrap_or(0))
}

fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn renamed_prefix(name: &str) -> String {
    format!(
        "{NEW_PREFIX}{}",
        name.strip_prefix(OLD_PREFIX).unwrap_or(name)
    )
}

/// The old-named tables the database still holds.
fn legacy_tables(client: &mut impl GenericClient) -> Result<Vec<&'static str>> {
    let mut legacy = Vec::new();
    for (from, _) in POSTGRES_TABLE_RENAMES {
        if table_exists(client, from)? {
            legacy.push(*from);
        }
    }
    Ok(legacy)
}

/// Errors while the database still holds an old-named table. Called first by each store's
/// schema setup.
pub(crate) fn require_current_names(client: &mut impl GenericClient) -> Result<()> {
    let legacy = legacy_tables(client)?;
    if legacy.is_empty() {
        Ok(())
    } else {
        Err(legacy_storage_refusal(&legacy).into())
    }
}

/// Gives the constraints and indexes of the renamed table `table` the new prefix. A primary key
/// or unique constraint renames the index behind it.
fn rename_prefixed_names(client: &mut impl GenericClient, table: &str) -> Result<()> {
    let constraints = client
        .query(
            "SELECT conname::text FROM pg_constraint
             WHERE conrelid = $1::text::regclass AND left(conname, 3) = $2
             ORDER BY conname",
            &[&table, &OLD_PREFIX],
        )
        .map_err(storage_error)?;
    for row in constraints {
        let old: String = row.get(0);
        client
            .batch_execute(&format!(
                "ALTER TABLE {} RENAME CONSTRAINT {} TO {}",
                quoted(table),
                quoted(&old),
                quoted(&renamed_prefix(&old))
            ))
            .map_err(storage_error)?;
    }

    let indexes = client
        .query(
            "SELECT c.relname::text FROM pg_index i
             JOIN pg_class c ON c.oid = i.indexrelid
             WHERE i.indrelid = $1::text::regclass AND left(c.relname, 3) = $2
             ORDER BY c.relname",
            &[&table, &OLD_PREFIX],
        )
        .map_err(storage_error)?;
    for row in indexes {
        let old: String = row.get(0);
        client
            .batch_execute(&format!(
                "ALTER INDEX {} RENAME TO {}",
                quoted(&old),
                quoted(&renamed_prefix(&old))
            ))
            .map_err(storage_error)?;
    }
    Ok(())
}

impl PostgresEventLedger {
    /// Renames the old-named tables of this database to their new names, in one transaction.
    /// With `dry_run` the same work runs and is rolled back, so the report is exactly what a
    /// real run would produce. Errors, changing nothing, when a table exists under both names or
    /// a table's row count differs afterwards. A database with nothing to rename reports
    /// nothing renamed.
    pub fn rename_legacy_storage(&self, dry_run: bool) -> Result<StorageRenameReport> {
        let mut report = StorageRenameReport::new("postgres", dry_run);
        let mut client = self.pool.get().map_err(storage_error)?;
        let mut transaction = client.transaction().map_err(storage_error)?;
        transaction
            .batch_execute("SELECT pg_advisory_xact_lock(hashtext('loomtracer_schema_init'));")
            .map_err(storage_error)?;

        for (from, to) in POSTGRES_TABLE_RENAMES {
            let has_old = table_exists(&mut transaction, from)?;
            let has_new = table_exists(&mut transaction, to)?;
            match (has_old, has_new) {
                (true, true) => return Err(both_names_exist(from, to).into()),
                (false, true) => report.already_current.push((*to).to_owned()),
                (false, false) => {}
                (true, false) => {
                    let rows_before = row_count(&mut transaction, from)?;
                    transaction
                        .batch_execute(&format!("ALTER TABLE {from} RENAME TO {to}"))
                        .map_err(storage_error)?;
                    rename_prefixed_names(&mut transaction, to)?;
                    let rows_after = row_count(&mut transaction, to)?;
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

        if dry_run {
            transaction.rollback().map_err(storage_error)?;
        } else {
            transaction.commit().map_err(storage_error)?;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
