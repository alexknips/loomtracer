// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

#[test]
fn domain_errors_convert_to_crate_error() {
    let ledger_error: LoomtracerError = LedgerError::Storage("disk full".to_owned()).into();
    assert!(matches!(ledger_error, LoomtracerError::Ledger(_)));

    let projector_error: LoomtracerError =
        ProjectorError::Projection("missing node".to_owned()).into();
    assert!(matches!(projector_error, LoomtracerError::Projector(_)));

    let command_error: LoomtracerError =
        CommandError::Invariant("actor_id is required".to_owned()).into();
    assert!(matches!(command_error, LoomtracerError::Command(_)));

    let query_error: LoomtracerError = QueryError::Execution("timeout".to_owned()).into();
    assert!(matches!(query_error, LoomtracerError::Query(_)));

    let cli_error: LoomtracerError = CliError::InvalidInput("--actor is empty".to_owned()).into();
    assert!(matches!(cli_error, LoomtracerError::Cli(_)));
}
