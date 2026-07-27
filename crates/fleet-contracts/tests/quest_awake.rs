// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    QUEST_AWAKE_MAX_DURATION_MS, QuestAwakeAction, QuestAwakeOperation, ValidateContract,
};

fn valid_operation() -> QuestAwakeOperation {
    serde_json::from_str(include_str!(
        "../../../fixtures/contracts/quest-awake-operation.valid.json"
    ))
    .expect("valid Quest awake fixture")
}

#[test]
fn duration_is_capped_at_exactly_eight_hours() {
    let mut operation = valid_operation();
    assert_eq!(operation.preview.duration_ms, QUEST_AWAKE_MAX_DURATION_MS);
    assert!(operation.validate().is_ok());

    operation.preview.duration_ms += 1;
    let codes = operation
        .validate()
        .expect_err("more than eight hours must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"invalid_duration".to_owned()));
    assert!(codes.contains(&"invocation_binding_mismatch".to_owned()));
    assert!(codes.contains(&"receipt_binding_mismatch".to_owned()));
}

#[test]
fn exact_policy_and_request_bindings_are_required() {
    let mut operation = valid_operation();
    operation.targets[0]
        .invocation
        .as_mut()
        .expect("invocation")
        .watchdog_interval_ms = 6_000;
    operation.targets[0]
        .receipt
        .as_mut()
        .expect("receipt")
        .request_id = "replacement-request".to_owned();
    let codes = operation
        .validate()
        .expect_err("changed invocation policy or request identity must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"invocation_binding_mismatch".to_owned()));
    assert!(codes.contains(&"receipt_binding_mismatch".to_owned()));
}

#[test]
fn stopping_watchdogs_does_not_claim_settings_were_restored() {
    let mut operation = valid_operation();
    let receipt = operation.targets[0].receipt.as_mut().expect("receipt");
    receipt.settings_left_unchanged = false;
    receipt.settings_restored = true;
    let codes = operation.targets[0]
        .receipt
        .as_ref()
        .expect("receipt")
        .validate()
        .expect_err("stop and restore are separate actions")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"readback_summary_mismatch".to_owned()));
}

#[test]
fn restore_requires_an_independent_restored_readback() {
    let mut operation = valid_operation();
    operation.preview.action = QuestAwakeAction::RestoreNormal;
    let invocation = operation.targets[0]
        .invocation
        .as_mut()
        .expect("invocation");
    invocation.action = QuestAwakeAction::RestoreNormal;
    let receipt = operation.targets[0].receipt.as_mut().expect("receipt");
    receipt.action = QuestAwakeAction::RestoreNormal;
    receipt.settings_left_unchanged = false;
    receipt.settings_restored = false;
    receipt.effective = true;
    let codes = operation.targets[0]
        .receipt
        .as_ref()
        .expect("receipt")
        .validate()
        .expect_err("restore without restored settings must fail")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"receipt_effect_mismatch".to_owned()));
}

#[test]
fn device_watchdog_effect_requires_fresh_matching_generation_readback() {
    let mut operation = valid_operation();
    operation.preview.action = QuestAwakeAction::StartDeviceWatchdog;
    let invocation = operation.targets[0]
        .invocation
        .as_mut()
        .expect("invocation");
    invocation.action = QuestAwakeAction::StartDeviceWatchdog;
    let receipt = operation.targets[0].receipt.as_mut().expect("receipt");
    receipt.action = QuestAwakeAction::StartDeviceWatchdog;
    receipt.settings_left_unchanged = false;
    receipt.device_watchdog_effective = true;
    receipt.device_watchdog.reported_active = true;
    receipt.device_watchdog.fresh = false;
    receipt.effective = true;
    let codes = operation.targets[0]
        .receipt
        .as_ref()
        .expect("receipt")
        .validate()
        .expect_err("stale watchdog evidence must not prove application")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"readback_summary_mismatch".to_owned()));
}

#[test]
fn summary_effects_cannot_contradict_raw_power_readback() {
    let mut operation = valid_operation();
    let receipt = operation.targets[0].receipt.as_mut().expect("receipt");
    receipt.power.stay_on = false;
    let codes = receipt
        .validate()
        .expect_err("summary stay-on cannot mask raw readback")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"readback_summary_mismatch".to_owned()));
}

#[test]
fn stop_cannot_mask_a_reported_active_device_watchdog() {
    let mut operation = valid_operation();
    let receipt = operation.targets[0].receipt.as_mut().expect("receipt");
    receipt.device_watchdog.reported_active = true;
    receipt.effective = true;
    let codes = receipt
        .validate()
        .expect_err("raw active watchdog must defeat stop proof")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"receipt_effect_mismatch".to_owned()));
}

#[test]
fn active_watchdog_rejects_the_empty_boot_identity_digest() {
    let mut operation = valid_operation();
    let receipt = operation.targets[0].receipt.as_mut().expect("receipt");
    receipt.action = QuestAwakeAction::StartDeviceWatchdog;
    receipt.settings_left_unchanged = false;
    receipt.device_watchdog.reported_active = true;
    receipt.device_watchdog.fresh = true;
    receipt.device_watchdog.generation = receipt.watchdog_generation.clone();
    receipt.device_watchdog.interval_ms = receipt.requested_watchdog_interval_ms;
    receipt.device_watchdog.last_poll_ms = receipt.observed_at_ms;
    receipt.device_watchdog.boot_id_sha256 =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned();
    receipt.device_watchdog_effective = true;
    receipt.effective = true;
    let codes = receipt
        .validate()
        .expect_err("empty boot identity cannot prove a reboot epoch")
        .into_iter()
        .map(|failure| failure.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"invalid_readback".to_owned()));
}
