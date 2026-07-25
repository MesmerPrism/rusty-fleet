// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use fleet_kiosk_adapter::{
    AdapterError, FleetKioskAdapter, HEADER_CONTENT_SHA256, HEADER_REQUEST_ID, HEADER_SIGNATURE,
    HEADER_TIMESTAMP, HttpMethod, KioskAdapterLimits, KioskHttpRequest, KioskHttpResponse,
    KioskShowControlsRequest, KioskTransport, RedirectPolicy, TransportRequestIdSource, sha256_hex,
    sign_request, sign_response,
};
use serde_json::{Value, json};

const KEY: &str = "0123-4567-89AB-CDEF";
const ENDPOINT: &str = "http://192.0.2.10:39873";
const ACTION_ID: &str = "action_0001";
const STARTED_AT_MS: i64 = 1_784_650_000_000;

#[derive(Clone)]
struct ResponseStep {
    status: u16,
    body: Vec<u8>,
    signed: bool,
    redirected: bool,
    response_request_id: Option<String>,
    declared_digest: Option<String>,
    signature: Option<String>,
    received_at_ms: Option<i64>,
}

impl ResponseStep {
    fn signed_json(value: Value) -> Result<Self, serde_json::Error> {
        Ok(Self {
            status: 200,
            body: serde_json::to_vec(&value)?,
            signed: true,
            redirected: false,
            response_request_id: None,
            declared_digest: None,
            signature: None,
            received_at_ms: None,
        })
    }

    fn unsigned_bytes(body: &[u8]) -> Self {
        Self {
            status: 200,
            body: body.to_vec(),
            signed: false,
            redirected: false,
            response_request_id: None,
            declared_digest: None,
            signature: None,
            received_at_ms: None,
        }
    }
}

struct ScriptedTransport {
    now_ms: i64,
    steps: VecDeque<ResponseStep>,
    observed: Vec<KioskHttpRequest>,
}

impl ScriptedTransport {
    fn new(steps: Vec<ResponseStep>) -> Self {
        Self {
            now_ms: STARTED_AT_MS,
            steps: steps.into(),
            observed: Vec::new(),
        }
    }
}

impl KioskTransport for ScriptedTransport {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }

    fn wait_until_ms(&mut self, not_before_ms: i64, deadline_at_ms: i64) -> Result<(), String> {
        if not_before_ms > deadline_at_ms {
            self.now_ms = not_before_ms;
            return Ok(());
        }
        self.now_ms = self.now_ms.max(not_before_ms);
        Ok(())
    }

    fn send(&mut self, request: KioskHttpRequest) -> Result<KioskHttpResponse, String> {
        validate_outgoing_request(&request, KEY)?;
        let step = self
            .steps
            .pop_front()
            .ok_or_else(|| "scripted transport ran out of responses".to_owned())?;
        let outgoing_request_id = request
            .headers
            .get(HEADER_REQUEST_ID)
            .ok_or_else(|| "outgoing request ID missing".to_owned())?
            .clone();
        let response_request_id = step
            .response_request_id
            .unwrap_or_else(|| outgoing_request_id.clone());
        let actual_digest = sha256_hex(&step.body);
        let declared_digest = step.declared_digest.unwrap_or(actual_digest);
        let mut headers = BTreeMap::new();
        if step.signed {
            let signature = match step.signature {
                Some(signature) => signature,
                None => sign_response(KEY, &response_request_id, step.status, &declared_digest)
                    .map_err(|error| error.to_string())?,
            };
            headers.insert(HEADER_REQUEST_ID.to_owned(), response_request_id);
            headers.insert(HEADER_CONTENT_SHA256.to_owned(), declared_digest);
            headers.insert(HEADER_SIGNATURE.to_owned(), signature);
        }
        self.now_ms = step.received_at_ms.unwrap_or(self.now_ms + 5);
        self.observed.push(request);
        Ok(KioskHttpResponse {
            status: step.status,
            headers,
            body: step.body,
            redirected: step.redirected,
            received_at_ms: self.now_ms,
        })
    }
}

struct RequestIds {
    ids: VecDeque<String>,
}

impl RequestIds {
    fn new(ids: &[&str]) -> Self {
        Self {
            ids: ids.iter().map(|value| (*value).to_owned()).collect(),
        }
    }
}

impl TransportRequestIdSource for RequestIds {
    fn next_transport_request_id(&mut self) -> Result<String, String> {
        self.ids
            .pop_front()
            .ok_or_else(|| "scripted request IDs exhausted".to_owned())
    }
}

fn validate_outgoing_request(request: &KioskHttpRequest, key: &str) -> Result<(), String> {
    if request.redirect_policy != RedirectPolicy::Deny
        || request.maximum_response_bytes > 512 * 1024
        || request.timeout_ms == 0
        || request.deadline_at_ms <= STARTED_AT_MS
        || request.url != format!("{ENDPOINT}{}", request.request_target)
    {
        return Err("outgoing bounds or endpoint were not preserved".to_owned());
    }
    let request_id = request
        .headers
        .get(HEADER_REQUEST_ID)
        .ok_or_else(|| "request ID header missing".to_owned())?;
    let timestamp = request
        .headers
        .get(HEADER_TIMESTAMP)
        .ok_or_else(|| "timestamp header missing".to_owned())?
        .parse::<i64>()
        .map_err(|_| "timestamp header invalid".to_owned())?;
    let declared_digest = request
        .headers
        .get(HEADER_CONTENT_SHA256)
        .ok_or_else(|| "digest header missing".to_owned())?;
    if *declared_digest != sha256_hex(&request.body) {
        return Err("outgoing digest mismatch".to_owned());
    }
    let expected_signature = sign_request(
        key,
        request.method.as_str(),
        &request.request_target,
        request_id,
        timestamp,
        declared_digest,
    )
    .map_err(|error| error.to_string())?;
    if request.headers.get(HEADER_SIGNATURE) != Some(&expected_signature) {
        return Err("outgoing signature mismatch".to_owned());
    }
    Ok(())
}

fn request() -> KioskShowControlsRequest {
    KioskShowControlsRequest {
        receipt_id: "receipt-0001".to_owned(),
        operation_id: "operation-0001".to_owned(),
        device_id: "device-0001".to_owned(),
        identity_revision: 7,
        endpoint: ENDPOINT.to_owned(),
        owner_action_request_id: ACTION_ID.to_owned(),
    }
}

fn status() -> Result<ResponseStep, serde_json::Error> {
    ResponseStep::signed_json(json!({
        "schema": "rusty.kiosk.direct_operator.v1",
        "accepted": true
    }))
}

fn invoke() -> Result<ResponseStep, serde_json::Error> {
    ResponseStep::signed_json(json!({
        "request_id": ACTION_ID,
        "accepted": true,
        "completed": false
    }))
}

fn pending() -> Result<ResponseStep, serde_json::Error> {
    ResponseStep::signed_json(json!({
        "request_id": ACTION_ID,
        "accepted": true,
        "completed": false
    }))
}

fn completed(controls_open: bool) -> Result<ResponseStep, serde_json::Error> {
    ResponseStep::signed_json(json!({
        "schema": "rusty.kiosk.cli_result.v1",
        "request_id": ACTION_ID,
        "command": "show-controls",
        "accepted": true,
        "completed": true,
        "recorded_at_ms": STARTED_AT_MS + 20,
        "state": {
            "controls_open": controls_open
        }
    }))
}

fn adapter() -> Result<FleetKioskAdapter, AdapterError> {
    FleetKioskAdapter::new(KioskAdapterLimits {
        maximum_polls: 3,
        poll_interval_ms: 10,
        operation_deadline_ms: 1_000,
        request_timeout_ms: 100,
        maximum_response_bytes: 512 * 1024,
    })
}

#[test]
fn executes_exact_signed_show_controls_flow_and_wraps_raw_owner_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    let final_step = completed(true)?;
    let expected_raw = final_step.body.clone();
    let mut transport = ScriptedTransport::new(vec![status()?, invoke()?, pending()?, final_step]);
    let mut ids = RequestIds::new(&["status001", "invoke01", "result01", "result02"]);

    let outcome = adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids)?;

    assert_eq!(outcome.poll_count, 2);
    assert_eq!(outcome.status_transport_request_id, "status001");
    assert_eq!(outcome.invoke_transport_request_id, "invoke01");
    assert_eq!(
        outcome.effective_receipt.owner_result_transport_request_id,
        "result02"
    );
    assert_eq!(outcome.effective_receipt.owner_action_request_id, ACTION_ID);
    assert!(outcome.effective_receipt.response_auth_verified);
    assert!(outcome.effective_receipt.owner_accepted);
    assert!(outcome.effective_receipt.owner_completed);
    assert!(outcome.effective_receipt.controls_open);
    assert_eq!(outcome.owner_receipt.raw_receipt, expected_raw);
    assert_eq!(
        outcome.owner_receipt.raw_receipt_sha256,
        sha256_hex(&outcome.owner_receipt.raw_receipt)
    );
    assert_eq!(
        outcome.owner_receipt.host_received_at_ms,
        outcome.effective_receipt.wrapped_at_ms
    );

    assert_eq!(transport.observed.len(), 4);
    assert_eq!(transport.observed[0].method, HttpMethod::Get);
    assert_eq!(transport.observed[0].request_target, "/v1/status");
    assert_eq!(transport.observed[1].method, HttpMethod::Post);
    assert_eq!(transport.observed[1].request_target, "/v1/kiosk/invoke");
    assert_eq!(
        transport.observed[2].request_target,
        format!("/v1/kiosk/result?request_id={ACTION_ID}")
    );
    assert_eq!(
        transport.observed[3].request_target,
        transport.observed[2].request_target
    );

    let invoke_body: Value = serde_json::from_slice(&transport.observed[1].body)?;
    assert_eq!(
        invoke_body,
        json!({"request_id": ACTION_ID, "command": "show-controls"})
    );
    assert!(invoke_body.get("value").is_none());

    let transport_ids: BTreeSet<_> = transport
        .observed
        .iter()
        .filter_map(|request| request.headers.get(HEADER_REQUEST_ID))
        .collect();
    assert_eq!(transport_ids.len(), 4);
    assert!(
        !transport_ids
            .iter()
            .any(|value| value.as_str() == ACTION_ID)
    );
    Ok(())
}

#[test]
fn restart_recovery_polls_the_same_owner_action_without_reinvoking()
-> Result<(), Box<dyn std::error::Error>> {
    let mut initial_transport = ScriptedTransport::new(vec![status()?, invoke()?]);
    let mut initial_ids = RequestIds::new(&["status001", "invoke01"]);
    let dispatch = adapter()?.invoke_show_controls(
        &request(),
        KEY,
        &mut initial_transport,
        &mut initial_ids,
    )?;
    assert_eq!(initial_transport.observed.len(), 2);

    let mut recovered_result = completed(true)?;
    recovered_result.received_at_ms = Some(STARTED_AT_MS + 25);
    let mut recovered_transport = ScriptedTransport::new(vec![recovered_result]);
    let mut recovered_ids = RequestIds::new(&["result01"]);
    let outcome = adapter()?.resume_show_controls_result(
        &request(),
        KEY,
        dispatch.deadline_at_ms,
        &mut recovered_transport,
        &mut recovered_ids,
    )?;
    assert_eq!(outcome.effective_receipt.owner_action_request_id, ACTION_ID);
    assert_eq!(recovered_transport.observed.len(), 1);
    assert_eq!(recovered_transport.observed[0].method, HttpMethod::Get);
    assert_eq!(
        recovered_transport.observed[0].request_target,
        format!("/v1/kiosk/result?request_id={ACTION_ID}")
    );
    Ok(())
}

#[test]
fn authenticates_response_bytes_before_parsing_json() -> Result<(), Box<dyn std::error::Error>> {
    let mut unsigned =
        ScriptedTransport::new(vec![ResponseStep::unsigned_bytes(b"{not valid json")]);
    let mut unsigned_ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut unsigned, &mut unsigned_ids),
        Err(AdapterError::MissingResponseHeader(HEADER_REQUEST_ID))
    );

    let mut signed_invalid = ResponseStep::unsigned_bytes(b"{not valid json");
    signed_invalid.signed = true;
    let mut signed = ScriptedTransport::new(vec![signed_invalid]);
    let mut signed_ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut signed, &mut signed_ids),
        Err(AdapterError::InvalidJson)
    );
    Ok(())
}

#[test]
fn rejects_redirect_digest_signature_and_size_damage() -> Result<(), Box<dyn std::error::Error>> {
    let mut redirect = status()?;
    redirect.status = 302;
    redirect.redirected = true;
    let mut transport = ScriptedTransport::new(vec![redirect]);
    let mut ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids),
        Err(AdapterError::RedirectRejected)
    );

    let mut bad_digest = status()?;
    bad_digest.declared_digest =
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned());
    let mut transport = ScriptedTransport::new(vec![bad_digest]);
    let mut ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids),
        Err(AdapterError::ResponseDigestMismatch)
    );

    let mut bad_signature = status()?;
    bad_signature.signature =
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned());
    let mut transport = ScriptedTransport::new(vec![bad_signature]);
    let mut ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids),
        Err(AdapterError::ResponseSignatureMismatch)
    );

    let mut too_large = ResponseStep::unsigned_bytes(&vec![b'x'; 512 * 1024 + 1]);
    too_large.signed = true;
    let mut transport = ScriptedTransport::new(vec![too_large]);
    let mut ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids),
        Err(AdapterError::ResponseTooLarge)
    );
    Ok(())
}

#[test]
fn rejects_mismatched_or_ineffective_owner_results() -> Result<(), Box<dyn std::error::Error>> {
    let damaged_results = [
        json!({
            "schema": "rusty.kiosk.cli_result.v1",
            "request_id": "wrong_0001",
            "command": "show-controls",
            "accepted": true,
            "completed": true,
            "recorded_at_ms": STARTED_AT_MS + 200,
            "state": {"controls_open": true}
        }),
        json!({
            "schema": "rusty.kiosk.cli_result.v1",
            "request_id": ACTION_ID,
            "command": "hide-controls",
            "accepted": true,
            "completed": true,
            "recorded_at_ms": STARTED_AT_MS + 200,
            "state": {"controls_open": true}
        }),
        json!({
            "schema": "rusty.kiosk.cli_result.v1",
            "request_id": ACTION_ID,
            "command": "show-controls",
            "accepted": true,
            "completed": true,
            "recorded_at_ms": STARTED_AT_MS + 200,
            "state": {"controls_open": false}
        }),
    ];
    for damaged in damaged_results {
        let mut transport = ScriptedTransport::new(vec![
            status()?,
            invoke()?,
            ResponseStep::signed_json(damaged)?,
        ]);
        let mut ids = RequestIds::new(&["status001", "invoke01", "result01"]);
        assert_eq!(
            adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids),
            Err(AdapterError::OwnerResultMismatch)
        );
    }

    let rejected = ResponseStep::signed_json(json!({
        "request_id": ACTION_ID,
        "accepted": false,
        "completed": true
    }))?;
    let mut transport = ScriptedTransport::new(vec![status()?, invoke()?, rejected]);
    let mut ids = RequestIds::new(&["status001", "invoke01", "result01"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut transport, &mut ids),
        Err(AdapterError::OwnerRejected)
    );
    Ok(())
}

#[test]
fn bounds_endpoints_ids_polls_and_deadlines() -> Result<(), Box<dyn std::error::Error>> {
    for endpoint in [
        "https://192.0.2.10:39873",
        "http://192.0.2.10:80",
        "http://192.0.2.10:39873/path",
        "http://user@192.0.2.10:39873",
    ] {
        let mut invalid = request();
        invalid.endpoint = endpoint.to_owned();
        let mut transport = ScriptedTransport::new(Vec::new());
        let mut ids = RequestIds::new(&[]);
        assert_eq!(
            adapter()?.execute_show_controls(&invalid, KEY, &mut transport, &mut ids),
            Err(AdapterError::InvalidEndpoint)
        );
    }

    let mut duplicate_transport =
        ScriptedTransport::new(vec![status()?, invoke()?, completed(true)?]);
    let mut ids = RequestIds::new(&["same_0001", "same_0001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut duplicate_transport, &mut ids),
        Err(AdapterError::DuplicateTransportRequestId)
    );

    let mut action_id_transport = ScriptedTransport::new(vec![status()?]);
    let mut ids = RequestIds::new(&[ACTION_ID]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut action_id_transport, &mut ids),
        Err(AdapterError::DuplicateTransportRequestId)
    );

    let mut polls = ScriptedTransport::new(vec![status()?, invoke()?, pending()?, pending()?]);
    let mut ids = RequestIds::new(&["status001", "invoke01", "result01", "result02"]);
    let poll_limited = FleetKioskAdapter::new(KioskAdapterLimits {
        maximum_polls: 2,
        ..KioskAdapterLimits::default()
    })?;
    assert_eq!(
        poll_limited.execute_show_controls(&request(), KEY, &mut polls, &mut ids),
        Err(AdapterError::PollLimitExceeded)
    );

    let mut late = status()?;
    late.received_at_ms = Some(STARTED_AT_MS + 1_001);
    let mut deadline = ScriptedTransport::new(vec![late]);
    let mut ids = RequestIds::new(&["status001"]);
    assert_eq!(
        adapter()?.execute_show_controls(&request(), KEY, &mut deadline, &mut ids),
        Err(AdapterError::DeadlineExceeded)
    );
    Ok(())
}
