// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::PathBuf;

fn config_path() -> Result<PathBuf, String> {
    let mut arguments = std::env::args_os();
    let executable = arguments.next().unwrap_or_else(|| "fleet-hub-local".into());
    match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(flag), Some(path), None) if flag == "--config" => Ok(PathBuf::from(path)),
        _ => Err(format!(
            "usage: {} --config <local-hub-config.json>",
            PathBuf::from(executable).display()
        )),
    }
}

#[tokio::main]
async fn main() {
    let result = async {
        let path = config_path()?;
        let config = fleet_hub_local::load_config(&path)?;
        let bind = config.bind.clone();
        eprintln!(
            "fleet-hub-local: listening on {bind}; enrolled credentials: {}",
            config.enrollments.len()
        );
        fleet_hub_local::serve(config).await
    }
    .await;
    if let Err(error) = result {
        eprintln!("fleet-hub-local: {error}");
        std::process::exit(1);
    }
}
