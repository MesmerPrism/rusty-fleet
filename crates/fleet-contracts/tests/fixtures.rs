// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    DeviceObservation, FleetCheckInClaims, FleetQuery, StreamDescriptor, ValidateContract,
};

#[test]
fn committed_valid_contract_fixtures_round_trip() {
    let observation: DeviceObservation = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/device-observation.valid.json"
    ))
    .expect("valid observation JSON");
    assert!(observation.validate().is_ok());

    let query: FleetQuery =
        serde_json::from_str(include_str!("../../../fixtures/contracts/query.valid.json"))
            .expect("valid query JSON");
    assert!(query.validate().is_ok());

    let stream: StreamDescriptor = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/stream-descriptor.valid.json"
    ))
    .expect("valid stream JSON");
    assert!(stream.validate().is_ok());

    let checkin: FleetCheckInClaims = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/checkin-claims.valid.json"
    ))
    .expect("valid check-in JSON");
    assert!(checkin.validate().is_ok());
}

#[test]
fn committed_damaged_checkin_fails_closed() {
    let checkin: FleetCheckInClaims = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/checkin-claims.damaged.json"
    ))
    .expect("damaged fixture remains syntactically valid JSON");
    let codes = checkin
        .validate()
        .expect_err("damaged check-in must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"invalid_checkin_window".to_owned()));
    assert!(codes.contains(&"invalid_manifold_proposal".to_owned()));
    assert!(codes.contains(&"invalid_revision".to_owned()));
}

#[test]
fn committed_damaged_observation_fails_closed() {
    let observation: DeviceObservation = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/device-observation.damaged.json"
    ))
    .expect("damaged fixture remains syntactically valid JSON");
    let codes = observation
        .validate()
        .expect_err("damaged observation must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"invalid_revision".to_owned()));
    assert!(codes.contains(&"invalid_battery".to_owned()));
    assert!(codes.contains(&"required_text".to_owned()));
}
