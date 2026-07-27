// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    QUEST_WIFI_ADB_RECEIPT_SCHEMA, QUEST_WIFI_ADB_TERMUX_PROOF_OWNER,
    QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA, QuestWifiAdbAction, QuestWifiAdbOwnerReceipt,
    QuestWifiAdbRouteMode, QuestWifiAdbTermuxProof, QuestWifiAdbWearerApproval,
    TERMUX_ADB_SHELL_IDENTITY, ValidateContract,
};

fn modern_request_receipt() -> QuestWifiAdbOwnerReceipt {
    QuestWifiAdbOwnerReceipt {
        schema: QUEST_WIFI_ADB_RECEIPT_SCHEMA.to_owned(),
        request_id: "request.wifi.1".to_owned(),
        operation_id: "operation.wifi.1".to_owned(),
        preview_id: "preview.wifi.1".to_owned(),
        device_id: "device.quest.1".to_owned(),
        identity_revision: 7,
        action: QuestWifiAdbAction::RequestWirelessAdb,
        route_mode: QuestWifiAdbRouteMode::ModernTls,
        request_delivered: true,
        kiosk_setting_applied: true,
        request_after_boot_enabled: None,
        wearer_approval: QuestWifiAdbWearerApproval::Pending,
        listener_discovered: false,
        effect_applied: true,
        outcome: "wireless_adb_request_applied".to_owned(),
        evidence_sha256: "11".repeat(32),
        observed_at_ms: 2_000,
    }
}

#[test]
fn modern_request_keeps_setting_prompt_listener_and_termux_facts_independent() {
    let receipt = modern_request_receipt();
    receipt.validate().expect("valid pending wearer request");
    let json = serde_json::to_value(&receipt).expect("receipt JSON");
    assert_eq!(json["wearer_approval"], "pending");
    assert_eq!(json["listener_discovered"], false);
    assert!(json.get("termux_usable").is_none());
    assert!(json.get("termux_shell_identity").is_none());
}

#[test]
fn classic_usb_tcpip_cannot_claim_kiosk_or_modern_prompt_facts() {
    let mut receipt = modern_request_receipt();
    receipt.action = QuestWifiAdbAction::EnableClassicTcpipFromUsb;
    receipt.route_mode = QuestWifiAdbRouteMode::ClassicTcpip;
    receipt.kiosk_setting_applied = false;
    receipt.wearer_approval = QuestWifiAdbWearerApproval::NotApplicable;
    receipt.listener_discovered = true;
    receipt.validate().expect("valid classic route");

    receipt.kiosk_setting_applied = true;
    assert!(receipt.validate().is_err());
}

#[test]
fn signed_termux_capability_requires_exact_shell_and_bounded_freshness() {
    let mut proof = QuestWifiAdbTermuxProof {
        schema: QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA.to_owned(),
        proof_id: "termux-proof-1".to_owned(),
        owner_id: QUEST_WIFI_ADB_TERMUX_PROOF_OWNER.to_owned(),
        device_id: "device.quest.1".to_owned(),
        identity_revision: 7,
        source_epoch: "agent-epoch-1".to_owned(),
        source_revision: 2,
        evidence_revision: 17,
        route_mode: QuestWifiAdbRouteMode::ModernTls,
        discovery_mode: "tls_nsd".to_owned(),
        listener_discovered: true,
        shell_identity: Some(TERMUX_ADB_SHELL_IDENTITY.to_owned()),
        available: true,
        evidence_sha256: "33".repeat(32),
        observed_at_ms: 2_000,
        fresh_until_ms: 62_000,
    };
    proof.validate().expect("valid signed capability fact");
    proof.shell_identity = Some("uid=10000(app)".to_owned());
    assert!(proof.validate().is_err());
    proof.shell_identity = Some(TERMUX_ADB_SHELL_IDENTITY.to_owned());
    proof.fresh_until_ms = proof.observed_at_ms + 300_001;
    assert!(proof.validate().is_err());
}

#[test]
fn unavailable_fact_is_explicit_and_cannot_retain_listener_or_shell() {
    let proof = QuestWifiAdbTermuxProof {
        schema: QUEST_WIFI_ADB_TERMUX_PROOF_SCHEMA.to_owned(),
        proof_id: "termux-proof-down-1".to_owned(),
        owner_id: QUEST_WIFI_ADB_TERMUX_PROOF_OWNER.to_owned(),
        device_id: "device.quest.1".to_owned(),
        identity_revision: 7,
        source_epoch: "agent-epoch-1".to_owned(),
        source_revision: 3,
        evidence_revision: 18,
        route_mode: QuestWifiAdbRouteMode::ModernTls,
        discovery_mode: "tls_mdns".to_owned(),
        listener_discovered: false,
        shell_identity: None,
        available: false,
        evidence_sha256: "44".repeat(32),
        observed_at_ms: 4_000,
        fresh_until_ms: 64_000,
    };
    proof
        .validate()
        .expect("valid unavailable supersession fact");
}
