// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `fleetctl` command projection over the same in-process API used by tests and
//! future UI consumers.

use fleet_contracts::{
    Comparison, FleetQuery, FleetQueryResult, FleetSummaryProjection, QueryExpression, QueryField,
    QueryValue, SavedView, SavedViewCollection, SavedViewMutationReceipt, SavedViewMutationRequest,
    SortDirection, SortKey,
};
use fleet_hub::{FleetApi, FleetHub, HubPolicy};
use fleet_simulator::{
    BASE_TIME_MS, MixedFreshnessFixture, ScenarioBuilder, mixed_freshness_fixture,
    supported_scale_fixtures,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperatorFixtureProjection {
    pub schema: String,
    pub profile: String,
    pub query_result: FleetQueryResult,
    pub summary: FleetSummaryProjection,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedViewRoundTripProjection {
    pub schema: String,
    pub saved: SavedViewMutationReceipt,
    pub collection: SavedViewCollection,
    pub restored_query_result: FleetQueryResult,
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
            "commands": [
                "list [count]",
                "inspect <device-id> [count]",
                "detail <device-id> [count]",
                "filter <text> [count]",
                "watch [count]",
                "scenario [count]",
                "operator-fixture mixed-freshness [count]",
                "saved-view-roundtrip [count]"
            ],
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
    if command == "operator-fixture" {
        let profile = arguments.get(1).ok_or_else(|| {
            CliFailure::new("missing_profile", "operator-fixture requires a profile")
        })?;
        if profile != "mixed-freshness" {
            return Err(CliFailure::new(
                "unknown_fixture_profile",
                format!("unknown operator fixture profile {profile}"),
            ));
        }
        return serde_json::to_value(mixed_operator_fixture(count)?)
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string()));
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
        "detail" => {
            let device_id = arguments.get(1).ok_or_else(|| {
                CliFailure::new("missing_device_id", "detail requires a device ID")
            })?;
            value(hub.detail(device_id, BASE_TIME_MS))
        }
        "filter" => {
            let text = arguments
                .get(1)
                .ok_or_else(|| CliFailure::new("missing_filter", "filter requires text"))?;
            value(hub.list(&text_query(text, count), BASE_TIME_MS))
        }
        "watch" => serde_json::to_value(hub.watch(0, count))
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string())),
        "saved-view-roundtrip" => serde_json::to_value(saved_view_roundtrip(hub, count)?)
            .map_err(|error| CliFailure::new("serialization_failed", error.to_string())),
        _ => Err(CliFailure::new(
            "unknown_command",
            format!("unknown command {command}"),
        )),
    }
}

pub fn saved_view_roundtrip(
    mut hub: FleetHub,
    count: usize,
) -> Result<SavedViewRoundTripProjection, CliFailure> {
    let mut query = default_query(count);
    query.query_id = "fleetctl.saved_view.needs_attention".to_owned();
    let view = SavedView {
        schema: "rusty.fleet.saved_view.v1".to_owned(),
        view_id: "view.needs_attention".to_owned(),
        name: "Needs attention".to_owned(),
        query,
        columns: vec![
            "device".to_owned(),
            "age".to_owned(),
            "route".to_owned(),
            "power".to_owned(),
            "application".to_owned(),
            "attention".to_owned(),
        ],
        density: "standard".to_owned(),
        grouping: None,
        restoration: fleet_contracts::NavigationRestoration {
            selected_device_id: Some("sim-00001".to_owned()),
            inspector_tab: Some("overview".to_owned()),
            scroll_anchor_device_id: Some("sim-00001".to_owned()),
            focused_region: Some("grid".to_owned()),
            collapsed_groups: Vec::new(),
        },
        schema_version: 1,
    };
    let saved = hub
        .upsert_saved_view(SavedViewMutationRequest {
            schema: "rusty.fleet.saved_view_mutation_request.v1".to_owned(),
            expected_revision: hub.saved_views().revision,
            view,
        })
        .map_err(|error| CliFailure::new("operation_failed", error.to_string()))?;
    let collection = hub.saved_views();
    let restored = hub
        .saved_view("view.needs_attention")
        .map_err(|error| CliFailure::new("operation_failed", error.to_string()))?;
    let restored_query_result = hub
        .list(&restored.query, BASE_TIME_MS)
        .map_err(|error| CliFailure::new("operation_failed", error.to_string()))?;
    Ok(SavedViewRoundTripProjection {
        schema: "rusty.fleet.saved_view_roundtrip.v1".to_owned(),
        saved,
        collection,
        restored_query_result,
    })
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
pub fn load_mixed_freshness_hub(count: usize) -> (FleetHub, i64) {
    let MixedFreshnessFixture {
        now_ms,
        observations,
        ..
    } = mixed_freshness_fixture(count);
    let mut hub = FleetHub::new(HubPolicy::default());
    for observation in observations {
        let accepted_at_ms = observation.received_time_ms;
        hub.accept_observation(observation, accepted_at_ms);
    }
    (hub, now_ms)
}

pub fn mixed_operator_fixture(count: usize) -> Result<OperatorFixtureProjection, CliFailure> {
    let (hub, now_ms) = load_mixed_freshness_hub(count);
    let query_result = hub
        .list(&default_query(count), now_ms)
        .map_err(|error| CliFailure::new("operation_failed", error.to_string()))?;
    Ok(OperatorFixtureProjection {
        schema: "rusty.fleet.operator_fixture.v1".to_owned(),
        profile: "mixed_freshness".to_owned(),
        query_result,
        summary: hub.summary(now_ms),
    })
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

    use super::{
        default_query, execute, load_hub, load_mixed_freshness_hub, saved_view_roundtrip,
        text_query,
    };

    #[test]
    fn commands_return_structured_json() {
        for args in [
            vec!["list".to_owned(), "4".to_owned()],
            vec!["inspect".to_owned(), "sim-00001".to_owned(), "4".to_owned()],
            vec!["detail".to_owned(), "sim-00001".to_owned(), "4".to_owned()],
            vec!["filter".to_owned(), "Quest 0001".to_owned(), "4".to_owned()],
            vec!["watch".to_owned(), "4".to_owned()],
            vec!["scenario".to_owned(), "4".to_owned()],
            vec![
                "operator-fixture".to_owned(),
                "mixed-freshness".to_owned(),
                "4".to_owned(),
            ],
            vec!["saved-view-roundtrip".to_owned(), "4".to_owned()],
        ] {
            assert!(execute(args).is_ok());
        }
    }

    #[test]
    fn operator_fixture_rejects_missing_and_unknown_profiles() {
        let missing = execute(vec!["operator-fixture".to_owned()]).expect_err("missing profile");
        assert_eq!(missing.code, "missing_profile");

        let unknown = execute(vec!["operator-fixture".to_owned(), "unknown".to_owned()])
            .expect_err("unknown profile");
        assert_eq!(unknown.code, "unknown_fixture_profile");
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

        let api_detail = serde_json::to_value(
            hub.detail("sim-00001", BASE_TIME_MS)
                .expect("local API detail"),
        )
        .expect("serialize detail");
        assert_eq!(
            execute(vec![
                "detail".to_owned(),
                "sim-00001".to_owned(),
                "4".to_owned()
            ])
            .expect("CLI detail"),
            api_detail
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

        let (mixed_hub, mixed_now_ms) = load_mixed_freshness_hub(4);
        let mixed_list = serde_json::to_value(
            mixed_hub
                .list(&default_query(4), mixed_now_ms)
                .expect("mixed local API list"),
        )
        .expect("serialize mixed list");
        let mixed_summary =
            serde_json::to_value(mixed_hub.summary(mixed_now_ms)).expect("serialize mixed summary");
        let mixed_cli = execute(vec![
            "operator-fixture".to_owned(),
            "mixed-freshness".to_owned(),
            "4".to_owned(),
        ])
        .expect("CLI mixed fixture");
        assert_eq!(mixed_cli["query_result"], mixed_list);
        assert_eq!(mixed_cli["summary"], mixed_summary);
        assert_eq!(mixed_cli["schema"], "rusty.fleet.operator_fixture.v1");
        assert_eq!(mixed_cli["profile"], "mixed_freshness");

        let saved_view_api =
            serde_json::to_value(saved_view_roundtrip(load_hub(4), 4).expect("saved-view API"))
                .expect("serialize saved-view API");
        assert_eq!(
            execute(vec!["saved-view-roundtrip".to_owned(), "4".to_owned()])
                .expect("CLI saved-view roundtrip"),
            saved_view_api
        );
    }
}
