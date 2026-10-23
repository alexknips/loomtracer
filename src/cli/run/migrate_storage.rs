//! CLI side of `loomtracer migrate-storage` (hivemind-x4xl7): rename the tables of a database
//! that predates the rename to Loomtracer, keeping every row. The rules, and why a store refuses
//! to start until this has run, are in [`crate::ledger`]'s `legacy_storage`.

use std::fmt::Write as _;
use std::path::Path;

use crate::cli::args::{Cli, MigrateStorageArgs};
use crate::cli::render::format_json_value;
use crate::error::CliError;
use crate::ledger::{SqliteEventLedger, StorageRenameReport};
use crate::Result;

pub(super) fn run_migrate_storage(cli: &Cli, args: &MigrateStorageArgs) -> Result<String> {
    let report = match cli.database_url.as_deref().filter(|url| !url.is_empty()) {
        Some(url) => rename_in_database(cli, url, args.dry_run)?,
        None => rename_in_local_ledger(&cli.loomtracer_dir, args.dry_run)?,
    };
    if cli.json {
        return format_json_value(true, &report);
    }
    Ok(render(&report))
}

fn rename_in_local_ledger(loomtracer_dir: &Path, dry_run: bool) -> Result<StorageRenameReport> {
    // `open` makes a ledger where there is none, and a typo in --loomtracer-dir would leave an
    // empty one behind for every later read to answer "0 decisions" from.
    if !SqliteEventLedger::exists_in(loomtracer_dir) {
        return Err(CliError::InvalidInput(format!(
            "`loomtracer migrate-storage` renames the tables of a ledger that exists, and there \
             is none under {}: nothing was changed and none was created. Check --loomtracer-dir \
             for a typo, or pass --database-url to rename a shared database",
            loomtracer_dir.display()
        ))
        .into());
    }
    SqliteEventLedger::rename_legacy_storage(loomtracer_dir, dry_run)
}

#[cfg(feature = "shared-backend-postgres")]
fn rename_in_database(cli: &Cli, url: &str, dry_run: bool) -> Result<StorageRenameReport> {
    // Only the events table is made on connecting; the stores that refuse an old-named database
    // are not opened here.
    let tenant = super::cli_tenant(cli)?;
    crate::ledger::PostgresEventLedger::connect(url, tenant.as_str())?
        .rename_legacy_storage(dry_run)
}

#[cfg(not(feature = "shared-backend-postgres"))]
fn rename_in_database(_cli: &Cli, _url: &str, _dry_run: bool) -> Result<StorageRenameReport> {
    Err(CliError::InvalidInput(
        "a database URL was configured but this binary was built without the \
         shared-backend-postgres feature"
            .to_owned(),
    )
    .into())
}

fn render(report: &StorageRenameReport) -> String {
    let (verb, tense) = if report.dry_run {
        ("Dry run", "would be renamed")
    } else {
        ("Storage names", "renamed")
    };
    if report.renamed.is_empty() {
        let current = report.already_current.len();
        return format!(
            "{verb}: nothing to rename in the {} storage; {current} tables already have their \
             Loomtracer names",
            report.backend
        );
    }
    let mut text = format!(
        "{verb}: {} tables {tense} in the {} storage, every row kept",
        report.renamed.len(),
        report.backend
    );
    for table in &report.renamed {
        let _ = write!(
            text,
            "\n  {} -> {}: {} rows before, {} after",
            table.from, table.to, table.rows_before, table.rows_after
        );
    }
    text
}
