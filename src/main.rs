use std::{env, process::ExitCode};

use server_rs::{config::Config, server};

fn main() -> ExitCode {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "server-rs".into());
    let config_path = match (args.next(), args.next()) {
        (Some(path), None) => path,
        (None, None) => "local/server.conf".to_owned(),
        _ => {
            eprintln!("Usage: {program} [config-file]");
            return ExitCode::from(2);
        }
    };

    let config = match Config::load(config_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Error: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = server_rs::server::initialize_logging(&config) {
        eprintln!("Error initializing logging: {error}");
        return ExitCode::FAILURE;
    }
    match server::run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("Server stopped: {error}");
            ExitCode::FAILURE
        }
    }
}
