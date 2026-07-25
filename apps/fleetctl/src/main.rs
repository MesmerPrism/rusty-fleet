// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::env;
use std::process::ExitCode;

use fleetctl::{
    LocalFleetOperationClient, execute, execute_with_operation_client, is_operation_command,
};

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let result = if arguments
        .first()
        .is_some_and(|command| is_operation_command(command))
    {
        LocalFleetOperationClient::from_env()
            .and_then(|mut client| execute_with_operation_client(arguments, &mut client))
    } else {
        execute(arguments)
    };
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => fail("serialization_failed", &error.to_string()),
        },
        Err(error) => fail(&error.code, &error.message),
    }
}

fn fail(code: &str, message: &str) -> ExitCode {
    let error = serde_json::json!({
        "schema": "rusty.fleet.error.v1",
        "code": code,
        "message": message,
    });
    eprintln!("{error}");
    ExitCode::FAILURE
}
