// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `fleetctl` command projection over the same in-process API used by tests and
//! future UI consumers.

use fleet_contracts::{
    Comparison, FleetQuery, QueryExpression, QueryField, QueryValue, SortDirection, SortKey,
};
use fleet_hub::{FleetApi, FleetHub, HubPolicy};
use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder, supported_scale_fixtures};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliFailure {
    pub code: String,
    pub message: String,
}

impl CliFailure {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

pub fn execute(arguments: Vec<String>) -> Result<serde_json::Value, CliFailure> {
    let command = arguments.first().map_or("help", String::as_str);
    if command == "help" {
        return Ok(serde_json::json!({
            "schema": "rusty.fleet.cli_help.v1",
            "commands": ["list [count]", "inspect <device-id> [count]", "filter <text> [count]", "watch [count]", "scenario [count]"],
            "scale_fixtures": supported_scale_fixtures()
        }));
    }
    let count = arguments
        .last()
        .filter(|value| value.chars().all(|character| character.is_ascii_digit()))
        .map_or(Ok(50_usize), |value| value.parse::<usize>())
        .map_err(|error| CliFailure::new("invalid_count", error.to_string()))?;
    if !supported_scale_fixtures().contains(&count) {
        return Err(CliFailure::new(
            "unsupported_fixture_size",
            format!("count must be one of {:?}", supported_scale_fixtures()),
        ));
    }
    let scenario = ScenarioBuilder::new(count).build();
    if command == "scenario" {
        return serde_json::to_value(scenario)
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string()));
    }
    let hub = load_hub(count);
    match command {
        "list" => value(hub.list(&default_query(count), BASE_TIME_MS)),
        "inspect" => {
            let device_id = arguments.get(1).ok_or_else(|| {
                CliFailure::new("missing_device_id", "inspect requires a device ID")
            })?;
            value(hub.inspect(device_id, BASE_TIME_MS))
        }
        "filter" => {
            let text = arguments
                .get(1)
                .ok_or_else(|| CliFailure::new("missing_filter", "filter requires text"))?;
            value(hub.list(&text_query(text, count), BASE_TIME_MS))
        }
        "watch" => serde_json::to_value(hub.watch(0, count))
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string())),
        _ => Err(CliFailure::new(
            "unknown_command",
            format!("unknown command {command}"),
        )),
    }
}

#[must_use]
pub fn load_hub(count: usize) -> FleetHub {
    let scenario = ScenarioBuilder::new(count).build();
    let mut hub = FleetHub::new(HubPolicy::default());
    for observation in scenario.initial {
        hub.accept_observation(observation, BASE_TIME_MS);
    }
    hub
}

#[must_use]
pub fn default_query(limit: usize) -> FleetQuery {
    FleetQuery {
        schema: "rusty.fleet.query.v1".to_owned(),
        query_id: "fleetctl".to_owned(),
        expression: None,
        sort: vec![SortKey {
            field: QueryField::DisplayName,
            direction: SortDirection::Ascending,
            qualifier: None,
        }],
        offset: 0,
        limit,
    }
}

#[must_use]
pub fn text_query(text: &str, limit: usize) -> FleetQuery {
    FleetQuery {
        expression: Some(QueryExpression::Or {
            expressions: vec![
                QueryExpression::Predicate {
                    field: QueryField::DisplayName,
                    comparison: Comparison::Contains,
                    value: Some(QueryValue::Text(text.to_owned())),
                    qualifier: None,
                },
                QueryExpression::Predicate {
                    field: QueryField::DeviceId,
                    comparison: Comparison::Contains,
                    value: Some(QueryValue::Text(text.to_owned())),
                    qualifier: None,
                },
            ],
        }),
        ..default_query(limit)
    }
}

fn value<T, E>(result: Result<T, E>) -> Result<serde_json::Value, CliFailure>
where
    T: serde::Serialize,
    E: ToString,
{
    let item = result.map_err(|error| CliFailure::new("operation_failed", error.to_string()))?;
    serde_json::to_value(item)
        .map_err(|error| CliFailure::new("serialization_failed", error.to_string()))
}

#[cfg(test)]
mod tests {
    use fleet_hub::FleetApi;
    use fleet_simulator::BASE_TIME_MS;

    use super::{default_query, execute, load_hub, text_query};

    #[test]
    fn commands_return_structured_json() {
        for args in [
            vec!["list".to_owned(), "4".to_owned()],
            vec!["inspect".to_owned(), "sim-00001".to_owned(), "4".to_owned()],
            vec!["filter".to_owned(), "Quest 0001".to_owned(), "4".to_owned()],
            vec!["watch".to_owned(), "4".to_owned()],
            vec!["scenario".to_owned(), "4".to_owned()],
        ] {
            assert!(execute(args).is_ok());
        }
    }

    #[test]
    fn cli_and_local_api_have_exact_projection_parity() {
        let hub = load_hub(4);
        let api_list = serde_json::to_value(
            hub.list(&default_query(4), BASE_TIME_MS)
                .expect("local API list"),
        )
        .expect("serialize list");
        assert_eq!(
            execute(vec!["list".to_owned(), "4".to_owned()]).expect("CLI list"),
            api_list
        );

        let api_inspect = serde_json::to_value(
            hub.inspect("sim-00001", BASE_TIME_MS)
                .expect("local API inspect"),
        )
        .expect("serialize inspect");
        assert_eq!(
            execute(vec![
                "inspect".to_owned(),
                "sim-00001".to_owned(),
                "4".to_owned()
            ])
            .expect("CLI inspect"),
            api_inspect
        );

        let api_filter = serde_json::to_value(
            hub.list(&text_query("Quest 0001", 4), BASE_TIME_MS)
                .expect("local API filter"),
        )
        .expect("serialize filter");
        assert_eq!(
            execute(vec![
                "filter".to_owned(),
                "Quest 0001".to_owned(),
                "4".to_owned()
            ])
            .expect("CLI filter"),
            api_filter
        );

        let api_watch = serde_json::to_value(hub.watch(0, 4)).expect("serialize watch");
        assert_eq!(
            execute(vec!["watch".to_owned(), "4".to_owned()]).expect("CLI watch"),
            api_watch
        );
    }
}
