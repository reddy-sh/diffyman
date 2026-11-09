//! `diffyman-server serve` and `diffyman-server keys ...`.

use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use diffyman_server::alloc::ServerAlloc;
use diffyman_server::config::Config;
use diffyman_server::{keys, serve};
use uuid::Uuid;

/// Large blocks go back to the system when freed, so the memory budgets
/// bound resident memory (`diffyman_server::alloc`).
#[global_allocator]
static GLOBAL: ServerAlloc = ServerAlloc;

#[derive(Parser)]
#[command(
    name = "diffyman-server",
    version,
    about = "Local REST service for the diffyman diff engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server: migrate the database, write the admin token file if
    /// missing, listen (127.0.0.1:8080 by default) and run jobs.
    Serve,
    /// Manage API keys (needs the admin token file and the database).
    Keys {
        #[command(subcommand)]
        command: KeysCommand,
    },
}

#[derive(Subcommand)]
enum KeysCommand {
    /// Create an API key and print it once on standard output.
    Create {
        /// A name for the key.
        #[arg(long)]
        name: String,
        /// Comma-separated scopes: read, write, delete (all by default).
        #[arg(long, value_delimiter = ',')]
        scopes: Vec<String>,
    },
    /// List API keys (never their secrets).
    List,
    /// Revoke an API key by ID.
    Revoke {
        /// The key's ID, as `keys list` shows it.
        id: Uuid,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = match Config::from_process() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("diffyman-server: configuration error: {error}");
            return ExitCode::from(2);
        }
    };
    for name in &config.unknown_vars {
        eprintln!("diffyman-server: warning: unknown variable {name}");
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("diffyman-server: cannot start: {error}");
            return ExitCode::from(1);
        }
    };
    let result = match cli.command {
        Command::Serve => runtime
            .block_on(serve::serve(config))
            .map_err(|e| e.to_string()),
        Command::Keys { command } => runtime
            .block_on(run_keys(&config, command))
            .map_err(|e| e.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("diffyman-server: error: {message}");
            ExitCode::from(1)
        }
    }
}

async fn run_keys(config: &Config, command: KeysCommand) -> Result<(), keys::KeysError> {
    let db = keys::open(config).await?;
    let mut out = std::io::stdout().lock();
    let result = match command {
        KeysCommand::Create { name, scopes } => {
            let (key, secret) = keys::create(&db, &name, &scopes).await?;
            writeln!(out, "{}", secret.expose())?;
            eprintln!(
                "diffyman-server: created key {} ({}…) with scopes {}; it is shown only once",
                key.id,
                key.prefix,
                key.scopes.join(",")
            );
            Ok(())
        }
        KeysCommand::List => {
            let list = diffyman_db::api_keys::list(db.pool()).await?;
            keys::write_list(&list, &mut out).map_err(keys::KeysError::from)
        }
        KeysCommand::Revoke { id } => {
            if keys::revoke(&db, id).await? {
                eprintln!("diffyman-server: revoked key {id}");
                Ok(())
            } else {
                Err(keys::KeysError::Invalid(format!(
                    "no active key with ID {id}"
                )))
            }
        }
    };
    db.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
