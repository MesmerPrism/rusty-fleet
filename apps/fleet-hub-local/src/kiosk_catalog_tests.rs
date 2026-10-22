use crate::KioskV2CatalogRuntimeProvider;
use fleet_kiosk_adapter::sha256_hex;
use std::sync::{Arc, Mutex as BlockingMutex};
// Synthetic host tests only; no owner process, network or headset.
use fleet_contracts::{
    KIOSK_CATALOG_SEARCH_SEMANTICS, KIOSK_OWNER_CATALOG_SNAPSHOT_SCHEMA,
    KIOSK_V2_CATALOG_REFRESH_REQUEST_SCHEMA, KioskOwnerCatalogEntry, KioskOwnerCatalogSnapshot,
    KioskV2CatalogRefreshRequest, owner_catalog_revision, owner_snapshot_digest,
};
use fleet_kiosk_adapter::{
    KioskEncryptedOperatorV2Provider, KioskV2CatalogExpectation, VerifiedKioskV2CatalogExchange,
};

struct CatalogTestProvider {
    started: Arc<tokio::sync::Notify>,
    gate: Option<Arc<(BlockingMutex<bool>, std::sync::Condvar)>>,
    owner: Arc<BlockingMutex<String>>,
    profile_id: String,
}
impl KioskV2CatalogRuntimeProvider for CatalogTestProvider {
    fn profile_id_for(&self, device: &str, revision: u64) -> Result<String, String> {
        if device == "device.quest.1" && revision == 1 {
            Ok(self.profile_id.clone())
        } else {
            Err("profile absent".to_owned())
        }
    }
}
fn test_catalog_exchange(
    expectation: &KioskV2CatalogExpectation,
    owner: &str,
) -> VerifiedKioskV2CatalogExchange {
    let observed = unix_time_ms().expect("time").max(expectation.issued_at_ms);
    let mut snapshot = KioskOwnerCatalogSnapshot {
        schema: KIOSK_OWNER_CATALOG_SNAPSHOT_SCHEMA.to_owned(),
        owner_epoch: owner.to_owned(),
        catalog_revision: "0".repeat(64),
        snapshot_digest: "0".repeat(64),
        observed_at_ms: observed,
        fresh_until_ms: observed + 300_000,
        tag_revision: "1".repeat(64),
        installed_observation_revision: "2".repeat(64),
        search_semantics: KIOSK_CATALOG_SEARCH_SEMANTICS.to_owned(),
        complete: true,
        truncated: false,
        total_candidate_count: 2,
        permission_limited_count: 0,
        entries: ["3", "4"]
            .into_iter()
            .map(|id| KioskOwnerCatalogEntry {
                catalog_entry_id: id.repeat(64),
                entry_digest: id.repeat(64),
                app_identity: id.repeat(64),
                disambiguator: id.repeat(12),
                label: "Study".to_owned(),
                package: None,
                tags: vec!["research".to_owned()],
                installed: true,
                launchable: true,
                source: "installed".to_owned(),
                signing_evidence: None,
            })
            .collect(),
    };
    snapshot.catalog_revision = owner_catalog_revision(&snapshot);
    snapshot.snapshot_digest = owner_snapshot_digest(&snapshot);
    VerifiedKioskV2CatalogExchange {
        request_envelope: b"synthetic request".to_vec(),
        response_envelope: b"synthetic encrypted owner response".to_vec(),
        owner_catalog_json: serde_json::to_vec(&snapshot).expect("owner JSON"),
        owner_grant_receipt: b"synthetic verified grant".to_vec(),
        response_request_id: expectation.request_id.clone(),
        replay_identity: sha256_hex(expectation.request_id.as_bytes()),
        capability_id: expectation.capability_id.clone(),
        capability_evidence_revision: expectation.capability_evidence_revision,
        grant_revision: 1,
        route_id: expectation.route_id.clone(),
        device_id: expectation.device_id.clone(),
        identity_revision: expectation.identity_revision,
        kiosk_owner_epoch: owner.to_owned(),
        issued_at_ms: expectation.issued_at_ms,
        expires_at_ms: expectation.expires_at_ms,
        scopes: expectation.scopes.clone(),
    }
}
impl KioskEncryptedOperatorV2Provider for CatalogTestProvider {
    fn verified_catalog_exchange(
        &mut self,
        expectation: &KioskV2CatalogExpectation,
    ) -> Result<VerifiedKioskV2CatalogExchange, String> {
        self.started.notify_one();
        if let Some(gate) = &self.gate {
            let (lock, ready) = &**gate;
            let mut done = lock.lock().expect("gate");
            while !*done {
                done = ready.wait(done).expect("gate wait");
            }
        }
        Ok(test_catalog_exchange(
            expectation,
            &self.owner.lock().expect("owner"),
        ))
    }
}
fn catalog_request() -> KioskV2CatalogRefreshRequest {
    KioskV2CatalogRefreshRequest {
        schema: KIOSK_V2_CATALOG_REFRESH_REQUEST_SCHEMA.to_owned(),
        device_id: "device.quest.1".to_owned(),
        identity_revision: 1,
    }
}
async fn accepted_catalog_state(
    gate: Option<Arc<(BlockingMutex<bool>, std::sync::Condvar)>>,
) -> (
    LocalHubState,
    LocalHubConfig,
    SigningKey,
    DottedId,
    Arc<tokio::sync::Notify>,
    Arc<BlockingMutex<String>>,
) {
    let now = unix_time_ms().expect("time");
    let key = SigningKey::from_bytes(&[83; 32]);
    let (config, key_id) = config(&key, now);
    let started = Arc::new(tokio::sync::Notify::new());
    let owner = Arc::new(BlockingMutex::new("OwnerEpoch_test_0001".to_owned()));
    let state = LocalHubState::from_config_with_kiosk_v2_catalog_provider(
        &config,
        now,
        CatalogTestProvider {
            started: started.clone(),
            gate,
            owner: owner.clone(),
            profile_id: "profile.quest.001".to_owned(),
        },
    )
    .expect("state");
    let response = router(state.clone())
        .oneshot(json_request(
            "/fleet/v1/checkins",
            serde_json::to_vec(&signed_checkin(&key, key_id.as_str(), now, 1)).expect("signed"),
        ))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    (state, config, key, key_id, started, owner)
}
#[tokio::test]
async fn catalog_read_is_atomic_durable_and_ambiguous_labels_keep_exact_identity() {
    let (state, config, _, _, _, owner) = accepted_catalog_state(None).await;
    let result = state
        .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
        .await
        .expect("read");
    assert_eq!(result.entry_count, 2);
    let query = fleet_contracts::KioskAppSearchRequest {
        schema: fleet_contracts::KIOSK_APP_SEARCH_REQUEST_SCHEMA.to_owned(),
        query: " research ".to_owned(),
        limit: 128,
    };
    let snapshot = {
        let runtime = state.runtime.lock().await;
        let matches = runtime
            .hub
            .kiosk_app_search(&query, result.observed_at_ms)
            .expect("search");
        assert_eq!(matches.matches.len(), 2);
        assert_ne!(
            matches.matches[0].app_identity,
            matches.matches[1].app_identity
        );
        assert_eq!(
            runtime
                .hub
                .kiosk_app_search(&query, result.fresh_until_ms + 1)
                .expect("stale")
                .matches
                .len(),
            0
        );
        runtime.hub.snapshot()
    };
    let restarted =
        LocalHubState::from_config(&config, unix_time_ms().expect("time")).expect("restart");
    assert_eq!(restarted.runtime.lock().await.hub.snapshot(), snapshot);
    *owner.lock().expect("owner") = "OwnerEpoch_changed_0002".to_owned();
    assert!(
        state
            .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
            .await
            .is_err()
    );
    assert_eq!(state.runtime.lock().await.hub.snapshot(), snapshot);
    let changed_profile = LocalHubState::from_config_with_kiosk_v2_catalog_provider(
        &config,
        unix_time_ms().expect("time"),
        CatalogTestProvider {
            started: Arc::new(tokio::sync::Notify::new()),
            gate: None,
            owner: Arc::new(BlockingMutex::new("OwnerEpoch_test_0001".to_owned())),
            profile_id: "profile.quest.002".to_owned(),
        },
    )
    .expect("restored different provider");
    assert!(
        changed_profile
            .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
            .await
            .expect_err("profile mismatch")
            .contains("setup_required")
    );
    assert_eq!(
        changed_profile.runtime.lock().await.hub.snapshot(),
        snapshot
    );
    drop(changed_profile);
    let mut damaged = serde_json::to_value(&snapshot).expect("JSON");
    damaged["seen_kiosk_catalog_replay_identities"] = serde_json::json!([]);
    assert!(
        FleetHub::restore(
            config.hub_policy.clone().into(),
            serde_json::from_value(damaged).expect("snapshot")
        )
        .is_err()
    );
    drop(restarted);
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_cancelled_caller_cleans_gate_and_ordinary_telemetry_does_not_invalidate_read() {
    let gate = Arc::new((BlockingMutex::new(false), std::sync::Condvar::new()));
    let (state, config, key, key_id, started, _) = accepted_catalog_state(Some(gate.clone())).await;
    let task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .synchronize_production_kiosk_catalog(
                    catalog_request(),
                    unix_time_ms().expect("time"),
                )
                .await
        }
    });
    started.notified().await;
    assert!(
        state
            .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
            .await
            .expect_err("busy")
            .contains("busy")
    );
    let mut signed = signed_checkin(&key, key_id.as_str(), unix_time_ms().expect("time"), 2);
    signed
        .claims
        .observation
        .capabilities
        .capabilities
        .get_mut("rusty-kiosk.direct-operator")
        .expect("capability")
        .evidence_revision = 1;
    signed.signature_hex = hex::encode(
        key.sign(&signed.claims.signing_bytes().expect("claims"))
            .to_bytes(),
    );
    let response = router(state.clone())
        .oneshot(json_request(
            "/fleet/v1/checkins",
            serde_json::to_vec(&signed).expect("signed"),
        ))
        .await
        .expect("checkin");
    assert_eq!(response.status(), StatusCode::OK);
    task.abort();
    *gate.0.lock().expect("gate") = true;
    gate.1.notify_all();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state
                .runtime
                .lock()
                .await
                .inflight_kiosk_catalog_refreshes
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cleanup bounded");
    let runtime = state.runtime.lock().await;
    assert!(
        runtime
            .hub
            .kiosk_catalog_association("device.quest.1")
            .is_some(),
        "same epoch ordinary telemetry permits exact read"
    );
    assert_eq!(runtime.kiosk_catalog_slots.available_permits(), 1);
    drop(runtime);
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_failed_persist_has_no_catalogue_replay_or_association_effect() {
    let (state, config, _, _, _, _) = accepted_catalog_state(None).await;
    let snapshot = state.runtime.lock().await.hub.snapshot();
    fs::remove_dir_all(&config.state_directory).expect("remove fixture state");
    fs::write(&config.state_directory, b"prevent state directory creation")
        .expect("block persistence");
    assert!(
        state
            .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
            .await
            .is_err()
    );
    assert_eq!(state.runtime.lock().await.hub.snapshot(), snapshot);
    drop(state);
    fs::remove_file(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_routes_are_closed_inert_and_absent_from_lan_router() {
    let now = unix_time_ms().expect("time");
    let key = SigningKey::from_bytes(&[84; 32]);
    let (mut config, _) = config(&key, now);
    let state = LocalHubState::from_config(&config, now).expect("state");
    let request = serde_json::to_vec(&catalog_request()).expect("request");
    assert_eq!(
        router(state.clone())
            .oneshot(json_request(
                "/fleet/v1/kiosk-catalog/refresh",
                request.clone()
            ))
            .await
            .expect("route")
            .status(),
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(
        checkin_router(state.clone())
            .oneshot(json_request("/fleet/v1/kiosk-catalog/refresh", request))
            .await
            .expect("LAN route")
            .status(),
        StatusCode::NOT_FOUND
    );
    for field in ["provider", "scopes", "barrier", "lease", "evidence"] {
        let mut request = serde_json::to_value(catalog_request()).expect("JSON");
        request[field] = serde_json::json!("injected");
        assert_eq!(
            router(state.clone())
                .oneshot(json_request(
                    "/fleet/v1/kiosk-catalog/refresh",
                    serde_json::to_vec(&request).expect("JSON")
                ))
                .await
                .expect("request")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    config.bind = "0.0.0.0:8741".to_owned();
    config.allow_non_loopback = true;
    assert!(
        LocalHubState::from_config_with_kiosk_v2_catalog_provider(
            &config,
            now,
            CatalogTestProvider {
                started: Arc::new(tokio::sync::Notify::new()),
                gate: None,
                owner: Arc::new(BlockingMutex::new("OwnerEpoch_test_0001".to_owned())),
                profile_id: "profile.quest.001".to_owned()
            }
        )
        .is_err()
    );
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_restore_rejects_binding_downgrade_and_nonloopback_readback() {
    let (state, mut config, _, _, _, _) = accepted_catalog_state(None).await;
    state
        .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
        .await
        .expect("read");
    let snapshot = state.runtime.lock().await.hub.snapshot();
    for remove_joins in [false, true] {
        let mut damaged = serde_json::to_value(&snapshot).expect("snapshot");
        damaged["kiosk_catalogs"]["device.quest.1"]["production_admission"] =
            serde_json::Value::Null;
        if remove_joins {
            damaged["kiosk_catalog_associations"] = serde_json::json!({});
            damaged["seen_kiosk_catalog_replay_identities"] = serde_json::json!([]);
        }
        assert!(
            FleetHub::restore(
                config.hub_policy.clone().into(),
                serde_json::from_value(damaged).expect("snapshot")
            )
            .is_err()
        );
    }
    config.bind = "0.0.0.0:8741".to_owned();
    config.allow_non_loopback = true;
    let restored = LocalHubState::from_config(&config, unix_time_ms().expect("time"))
        .expect("restored with provider absent");
    let app = router(restored.clone());
    assert_eq!(
        app.clone()
            .oneshot(json_request(
                "/fleet/v1/kiosk-apps/search",
                serde_json::to_vec(&fleet_contracts::KioskAppSearchRequest {
                    schema: fleet_contracts::KIOSK_APP_SEARCH_REQUEST_SCHEMA.to_owned(),
                    query: "study".to_owned(),
                    limit: 10
                })
                .expect("query")
            ))
            .await
            .expect("search")
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri(format!(
                    "/fleet/v1/kiosk-apps/{}/candidates",
                    "3".repeat(64)
                ))
                .body(Body::empty())
                .expect("request")
        )
        .await
        .expect("candidates")
        .status(),
        StatusCode::FORBIDDEN
    );
    drop(restored);
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_admission_resamples_time_after_waiting_for_runtime_lock() {
    let gate = Arc::new((BlockingMutex::new(false), std::sync::Condvar::new()));
    let (state, config, _, _, started, _) = accepted_catalog_state(Some(gate.clone())).await;
    {
        let mut runtime = state.runtime.lock().await;
        let mut observation = signed_checkin(
            &SigningKey::from_bytes(&[83; 32]),
            "unused",
            unix_time_ms().expect("time"),
            2,
        )
        .claims
        .observation;
        observation.received_time_ms = observation.source_time_ms;
        observation
            .capabilities
            .capabilities
            .get_mut("rusty-kiosk.direct-operator")
            .expect("capability")
            .fresh_until_ms = unix_time_ms().expect("time") + 200;
        assert!(matches!(
            runtime
                .hub
                .accept_observation(observation, unix_time_ms().expect("time")),
            fleet_hub::ObservationDecision::Accepted { .. }
        ));
    }
    let task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .synchronize_production_kiosk_catalog(
                    catalog_request(),
                    unix_time_ms().expect("time"),
                )
                .await
        }
    });
    started.notified().await;
    let runtime = state.runtime.lock().await;
    let prior = runtime.hub.snapshot();
    *gate.0.lock().expect("gate") = true;
    gate.1.notify_all();
    tokio::time::sleep(Duration::from_millis(250)).await;
    drop(runtime);
    assert!(task.await.expect("task").is_err());
    assert_eq!(state.runtime.lock().await.hub.snapshot(), prior);
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}

#[tokio::test]
async fn catalog_unknown_expired_and_changed_target_reads_have_no_durable_effect() {
    let (state, config, _, _, _, _) = accepted_catalog_state(None).await;
    let prior = state.runtime.lock().await.hub.snapshot();
    let mut unknown = catalog_request();
    unknown.device_id = "unknown.peer".to_owned();
    assert!(
        state
            .synchronize_production_kiosk_catalog(unknown, unix_time_ms().expect("time"))
            .await
            .is_err()
    );
    let mut wrong = catalog_request();
    wrong.identity_revision += 1;
    assert!(
        state
            .synchronize_production_kiosk_catalog(wrong, unix_time_ms().expect("time"))
            .await
            .is_err()
    );
    assert!(
        state
            .synchronize_production_kiosk_catalog(
                catalog_request(),
                unix_time_ms().expect("time") + 700_000
            )
            .await
            .is_err()
    );
    assert_eq!(state.runtime.lock().await.hub.snapshot(), prior);
    let mut old = serde_json::to_value(&prior).expect("snapshot");
    for key in [
        "kiosk_catalogs",
        "kiosk_catalog_associations",
        "seen_kiosk_catalog_replay_identities",
    ] {
        old.as_object_mut().expect("snapshot object").remove(key);
    }
    assert_eq!(
        FleetHub::restore(
            config.hub_policy.clone().into(),
            serde_json::from_value(old).expect("old snapshot")
        )
        .expect("backcompat")
        .snapshot(),
        prior
    );
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_expired_active_credential_refuses_before_provider_read() {
    let now = unix_time_ms().expect("time");
    let key = SigningKey::from_bytes(&[85; 32]);
    let (mut config, key_id) = config(&key, now);
    config.enrollments[0].credential.expires_at_ms = u64::try_from(now + 500).expect("expiry");
    let started = Arc::new(tokio::sync::Notify::new());
    let state = LocalHubState::from_config_with_kiosk_v2_catalog_provider(
        &config,
        now,
        CatalogTestProvider {
            started: started.clone(),
            gate: None,
            owner: Arc::new(BlockingMutex::new("OwnerEpoch_test_0001".to_owned())),
            profile_id: "profile.quest.001".to_owned(),
        },
    )
    .expect("state");
    assert_eq!(
        router(state.clone())
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed_checkin(&key, key_id.as_str(), now, 1)).expect("signed")
            ))
            .await
            .expect("checkin")
            .status(),
        StatusCode::OK
    );
    let prior = state.runtime.lock().await.hub.snapshot();
    tokio::time::sleep(Duration::from_millis(550)).await;
    let error = state
        .synchronize_production_kiosk_catalog(catalog_request(), unix_time_ms().expect("time"))
        .await
        .expect_err("credential expired");
    assert!(error.contains("credential absent or expired"), "{error}");
    assert_eq!(state.runtime.lock().await.hub.snapshot(), prior);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), started.notified())
            .await
            .is_err()
    );
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
#[tokio::test]
async fn catalog_target_capability_change_during_provider_read_refuses() {
    let gate = Arc::new((BlockingMutex::new(false), std::sync::Condvar::new()));
    let (state, config, key, key_id, started, _) = accepted_catalog_state(Some(gate.clone())).await;
    let task = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .synchronize_production_kiosk_catalog(
                    catalog_request(),
                    unix_time_ms().expect("time"),
                )
                .await
        }
    });
    started.notified().await;
    let signed = signed_checkin(&key, key_id.as_str(), unix_time_ms().expect("time"), 2);
    assert_eq!(
        router(state.clone())
            .oneshot(json_request(
                "/fleet/v1/checkins",
                serde_json::to_vec(&signed).expect("signed")
            ))
            .await
            .expect("checkin")
            .status(),
        StatusCode::OK
    );
    let prior = state.runtime.lock().await.hub.snapshot();
    *gate.0.lock().expect("gate") = true;
    gate.1.notify_all();
    assert!(
        task.await
            .expect("task")
            .expect_err("target changed")
            .contains("target changed")
    );
    assert_eq!(state.runtime.lock().await.hub.snapshot(), prior);
    assert_eq!(
        state
            .runtime
            .lock()
            .await
            .kiosk_catalog_slots
            .available_permits(),
        1
    );
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}

#[tokio::test]
async fn catalog_damaged_latest_slot_falls_back_and_incomplete_coverage_is_truthful() {
    let (state, config, _, _, _, _) = accepted_catalog_state(None).await;
    let now = unix_time_ms().expect("time");
    state
        .synchronize_production_kiosk_catalog(catalog_request(), now)
        .await
        .expect("catalogue");
    let mut snapshot =
        serde_json::to_value(state.runtime.lock().await.hub.snapshot()).expect("snapshot");
    let record = &mut snapshot["kiosk_catalogs"]["device.quest.1"];
    record["snapshot"]["complete"] = serde_json::json!(false);
    record["snapshot"]["permission_limited_count"] = serde_json::json!(1);
    // This is deliberately synthetic engine evidence; it exercises truthful
    // coverage after a valid strictly authenticated wrapper has been retained.
    let synthetic = FleetHub::restore(
        config.hub_policy.clone().into(),
        serde_json::from_value(snapshot).expect("snapshot"),
    )
    .expect("incomplete snapshot");
    let query = fleet_contracts::KioskAppSearchRequest {
        schema: fleet_contracts::KIOSK_APP_SEARCH_REQUEST_SCHEMA.to_owned(),
        query: "study".to_owned(),
        limit: 128,
    };
    let result = synthetic
        .kiosk_app_search(&query, unix_time_ms().expect("time"))
        .expect("coverage");
    assert_eq!(result.coverage.permission_limited, 1);
    assert!(result.matches.is_empty());
    let generation = state.runtime.lock().await.state_store.generation;
    let slot = state_slot_path(&config.state_directory, generation % 2);
    fs::write(slot, b"damaged latest slot").expect("damage fixture");
    let fallback =
        LocalHubState::from_config(&config, unix_time_ms().expect("time")).expect("fallback");
    assert_eq!(fallback.runtime.lock().await.hub.device_count(), 1);
    assert!(
        fallback
            .runtime
            .lock()
            .await
            .hub
            .kiosk_catalog_association("device.quest.1")
            .is_none()
    );
    drop(fallback);
    drop(state);
    fs::remove_dir_all(config.state_directory).expect("cleanup");
}
