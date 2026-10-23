//! The connection manager every Postgres store builds its pool on.
//!
//! r2d2 reports a pool that cannot connect as `timed out waiting for connection: <last connect
//! error>`, and `postgres::Error` displays a server refusal as just `db error`: the server's own
//! words ("password authentication failed for user ...") are in its cause, which `Display` leaves
//! out. A cell whose role has a different password than its `.env` says (hivemind-r9cs1) then
//! failed with an error that never said authentication failed. [`PgConnectionManager`] puts the
//! server's message back into the error r2d2 keeps.

use std::error::Error as _;
use std::fmt;

use postgres::{Client, Config};
use postgres_native_tls::MakeTlsConnector;
use r2d2::ManageConnection;
use r2d2_postgres::PostgresConnectionManager;

/// `r2d2_postgres`'s manager with connect errors that carry what the server said.
pub struct PgConnectionManager(PostgresConnectionManager<MakeTlsConnector>);

impl PgConnectionManager {
    pub fn new(config: Config, tls: MakeTlsConnector) -> Self {
        Self(PostgresConnectionManager::new(config, tls))
    }
}

/// A `postgres::Error` that displays the server's message and the causes under it.
#[derive(Debug)]
pub struct PgConnectError(postgres::Error);

impl fmt::Display for PgConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)?;
        match self.0.as_db_error() {
            Some(db) => write!(formatter, ": {}: {}", db.severity(), db.message()),
            None => {
                let mut cause = self.0.source();
                while let Some(error) = cause {
                    write!(formatter, ": {error}")?;
                    cause = error.source();
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for PgConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl ManageConnection for PgConnectionManager {
    type Connection = Client;
    type Error = PgConnectError;

    fn connect(&self) -> Result<Client, PgConnectError> {
        self.0.connect().map_err(PgConnectError)
    }

    fn is_valid(&self, client: &mut Client) -> Result<(), PgConnectError> {
        self.0.is_valid(client).map_err(PgConnectError)
    }

    fn has_broken(&self, client: &mut Client) -> bool {
        self.0.has_broken(client)
    }
}

#[cfg(test)]
mod tests;
