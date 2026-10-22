use fleet_contracts::{KioskV2CatalogRefreshRequest, ValidateContract};
#[test]
fn refresh_request_cannot_inject_provider_or_authority() {
    let raw = r#"{"schema":"rusty.fleet.kiosk_v2_catalog_refresh_request.v1","device_id":"test-device","identity_revision":1}"#;
    assert!(
        serde_json::from_str::<KioskV2CatalogRefreshRequest>(raw)
            .expect("request")
            .validate()
            .is_ok()
    );
    for field in [
        "provider",
        "scopes",
        "barrier",
        "lease",
        "evidence",
        "profile_id",
    ] {
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("JSON");
        value[field] = serde_json::json!("injected");
        assert!(serde_json::from_value::<KioskV2CatalogRefreshRequest>(value).is_err());
    }
}
