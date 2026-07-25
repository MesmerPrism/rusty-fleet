// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded adapter for Rusty Kiosk's wearer-enabled signed direct link.
//!
//! This crate owns no socket implementation. Callers inject a transport that
//! must honor the explicit no-redirect, response-size, timeout, and deadline
//! fields on every request. The adapter verifies signed response bytes before
//! parsing JSON or projecting an effective Fleet receipt.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

use fleet_contracts::{
    KIOSK_CLI_RESULT_SCHEMA, KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS,
    KIOSK_DIRECT_OPERATOR_PORT, KIOSK_DIRECT_OPERATOR_SCHEMA, KIOSK_SHOW_CONTROLS_COMMAND,
    KioskEffectiveReceipt, KioskOwnerContractBinding, ValidateContract,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const STATUS_TARGET: &str = "/v1/status";
pub const INVOKE_TARGET: &str = "/v1/kiosk/invoke";
pub const RESULT_TARGET: &str = "/v1/kiosk/result";
pub const HEADER_REQUEST_ID: &str = "x-rusty-request-id";
pub const HEADER_TIMESTAMP: &str = "x-rusty-timestamp";
pub const HEADER_CONTENT_SHA256: &str = "x-rusty-content-sha256";
pub const HEADER_SIGNATURE: &str = "x-rusty-signature";
pub const MAX_OWNER_JSON_BYTES: usize = 512 * 1024;
pub const MAX_INVOKE_BODY_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectPolicy {
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskHttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub request_target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub redirect_policy: RedirectPolicy,
    pub maximum_response_bytes: usize,
    pub timeout_ms: u64,
    pub deadline_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub redirected: bool,
    pub received_at_ms: i64,
}

pub trait KioskTransport {
    fn now_ms(&self) -> i64;

    fn wait_until_ms(&mut self, not_before_ms: i64, deadline_at_ms: i64) -> Result<(), String>;

    fn send(&mut self, request: KioskHttpRequest) -> Result<KioskHttpResponse, String>;
}

pub trait TransportRequestIdSource {
    fn next_transport_request_id(&mut self) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskAdapterLimits {
    pub maximum_polls: u16,
    pub poll_interval_ms: u64,
    pub operation_deadline_ms: u64,
    pub request_timeout_ms: u64,
    pub maximum_response_bytes: usize,
}

impl Default for KioskAdapterLimits {
    fn default() -> Self {
        Self {
            maximum_polls: 20,
            poll_interval_ms: 250,
            operation_deadline_ms: 30_000,
            request_timeout_ms: 5_000,
            maximum_response_bytes: MAX_OWNER_JSON_BYTES,
        }
    }
}

impl KioskAdapterLimits {
    fn validate(&self) -> Result<(), AdapterError> {
        if self.maximum_polls == 0
            || self.maximum_polls > 64
            || self.poll_interval_ms == 0
            || self.poll_interval_ms > 5_000
            || self.operation_deadline_ms == 0
            || self.operation_deadline_ms
                > u64::from(KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS) * 1_000
            || self.request_timeout_ms == 0
            || self.request_timeout_ms > self.operation_deadline_ms
            || self.maximum_response_bytes == 0
            || self.maximum_response_bytes > MAX_OWNER_JSON_BYTES
        {
            return Err(AdapterError::InvalidLimits);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskShowControlsRequest {
    pub receipt_id: String,
    pub operation_id: String,
    pub device_id: String,
    pub identity_revision: u64,
    pub endpoint: String,
    pub owner_action_request_id: String,
}

impl KioskShowControlsRequest {
    fn validate(&self) -> Result<(), AdapterError> {
        if self.receipt_id.trim().is_empty()
            || self.operation_id.trim().is_empty()
            || self.device_id.trim().is_empty()
            || self.identity_revision == 0
            || !is_request_id(&self.owner_action_request_id)
        {
            return Err(AdapterError::InvalidRequest);
        }
        validate_kiosk_endpoint(&self.endpoint)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawOwnerReceiptEvidence {
    pub raw_receipt_sha256: String,
    pub raw_receipt: Vec<u8>,
    pub host_received_at_ms: i64,
    pub result_transport_request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskShowControlsOutcome {
    pub effective_receipt: KioskEffectiveReceipt,
    pub owner_receipt: RawOwnerReceiptEvidence,
    pub status_transport_request_id: String,
    pub invoke_transport_request_id: String,
    pub poll_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskShowControlsDispatch {
    pub deadline_at_ms: i64,
    pub status_transport_request_id: String,
    pub invoke_transport_request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KioskShowControlsPollOutcome {
    pub effective_receipt: KioskEffectiveReceipt,
    pub owner_receipt: RawOwnerReceiptEvidence,
    pub poll_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterError {
    InvalidLimits,
    InvalidRequest,
    InvalidEndpoint,
    InvalidTransportRequestId,
    DuplicateTransportRequestId,
    DeadlineExceeded,
    PollLimitExceeded,
    Transport(String),
    RedirectRejected,
    ResponseTooLarge,
    MissingResponseHeader(&'static str),
    ResponseRequestIdMismatch,
    ResponseDigestMismatch,
    ResponseSignatureMismatch,
    UnexpectedHttpStatus(u16),
    InvalidJson,
    OwnerContractMismatch,
    OwnerInvokeMismatch,
    OwnerResultMismatch,
    OwnerRejected,
    EffectiveReceiptInvalid,
}

impl Display for AdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("Kiosk adapter limits are invalid"),
            Self::InvalidRequest => formatter.write_str("Kiosk show-controls request is invalid"),
            Self::InvalidEndpoint => formatter.write_str("Kiosk endpoint is invalid"),
            Self::InvalidTransportRequestId => {
                formatter.write_str("transport request ID is invalid")
            }
            Self::DuplicateTransportRequestId => formatter
                .write_str("transport request ID was reused or matched the owner action ID"),
            Self::DeadlineExceeded => formatter.write_str("Kiosk operation deadline was exceeded"),
            Self::PollLimitExceeded => formatter.write_str("Kiosk result poll limit was exceeded"),
            Self::Transport(message) => write!(formatter, "Kiosk transport failed: {message}"),
            Self::RedirectRejected => formatter.write_str("Kiosk redirect was rejected"),
            Self::ResponseTooLarge => formatter.write_str("Kiosk response exceeded its size bound"),
            Self::MissingResponseHeader(header) => {
                write!(formatter, "Kiosk signed response omitted {header}")
            }
            Self::ResponseRequestIdMismatch => {
                formatter.write_str("Kiosk response transport request ID did not match")
            }
            Self::ResponseDigestMismatch => {
                formatter.write_str("Kiosk response body digest did not match")
            }
            Self::ResponseSignatureMismatch => {
                formatter.write_str("Kiosk response signature did not match")
            }
            Self::UnexpectedHttpStatus(status) => {
                write!(formatter, "Kiosk returned unexpected HTTP status {status}")
            }
            Self::InvalidJson => formatter.write_str("Kiosk response JSON is invalid"),
            Self::OwnerContractMismatch => {
                formatter.write_str("Kiosk status did not match direct_operator.v1")
            }
            Self::OwnerInvokeMismatch => {
                formatter.write_str("Kiosk invoke acknowledgement did not match the action request")
            }
            Self::OwnerResultMismatch => {
                formatter.write_str("Kiosk result did not match show-controls")
            }
            Self::OwnerRejected => formatter.write_str("Kiosk rejected show-controls"),
            Self::EffectiveReceiptInvalid => {
                formatter.write_str("projected Fleet effective receipt is invalid")
            }
        }
    }
}

impl std::error::Error for AdapterError {}

#[derive(Clone, Debug)]
pub struct FleetKioskAdapter {
    limits: KioskAdapterLimits,
}

impl FleetKioskAdapter {
    pub fn new(limits: KioskAdapterLimits) -> Result<Self, AdapterError> {
        limits.validate()?;
        Ok(Self { limits })
    }

    pub fn execute_show_controls<T, I>(
        &self,
        request: &KioskShowControlsRequest,
        pairing_key: &str,
        transport: &mut T,
        request_ids: &mut I,
    ) -> Result<KioskShowControlsOutcome, AdapterError>
    where
        T: KioskTransport,
        I: TransportRequestIdSource,
    {
        request.validate()?;
        if pairing_key.is_empty() {
            return Err(AdapterError::InvalidRequest);
        }
        let owner_contract = KioskOwnerContractBinding::show_controls_v1();
        if owner_contract.validate().is_err() {
            return Err(AdapterError::OwnerContractMismatch);
        }
        let started_at_ms = transport.now_ms();
        let deadline_delta = i64::try_from(self.limits.operation_deadline_ms)
            .map_err(|_| AdapterError::InvalidLimits)?;
        let deadline_at_ms = started_at_ms
            .checked_add(deadline_delta)
            .ok_or(AdapterError::InvalidLimits)?;
        let mut used_transport_ids = BTreeSet::new();

        let status_transport_id = next_transport_id(
            request_ids,
            &request.owner_action_request_id,
            &mut used_transport_ids,
        )?;
        let status_response = self.send_signed(
            transport,
            pairing_key,
            &request.endpoint,
            HttpMethod::Get,
            STATUS_TARGET,
            Vec::new(),
            &status_transport_id,
            deadline_at_ms,
        )?;
        let status: OwnerStatus =
            serde_json::from_slice(&status_response.body).map_err(|_| AdapterError::InvalidJson)?;
        if !status.accepted || status.schema != KIOSK_DIRECT_OPERATOR_SCHEMA {
            return Err(AdapterError::OwnerContractMismatch);
        }

        let invoke_transport_id = next_transport_id(
            request_ids,
            &request.owner_action_request_id,
            &mut used_transport_ids,
        )?;
        let invoke_body = serde_json::to_vec(&OwnerInvoke {
            request_id: &request.owner_action_request_id,
            command: KIOSK_SHOW_CONTROLS_COMMAND,
        })
        .map_err(|_| AdapterError::InvalidJson)?;
        if invoke_body.len() > MAX_INVOKE_BODY_BYTES {
            return Err(AdapterError::InvalidRequest);
        }
        let invoke_response = self.send_signed(
            transport,
            pairing_key,
            &request.endpoint,
            HttpMethod::Post,
            INVOKE_TARGET,
            invoke_body,
            &invoke_transport_id,
            deadline_at_ms,
        )?;
        let invoke: OwnerInvokeResponse =
            serde_json::from_slice(&invoke_response.body).map_err(|_| AdapterError::InvalidJson)?;
        if !invoke.accepted
            || invoke.completed
            || invoke.request_id != request.owner_action_request_id
        {
            return Err(AdapterError::OwnerInvokeMismatch);
        }

        let result_target = format!(
            "{RESULT_TARGET}?request_id={}",
            request.owner_action_request_id
        );
        let mut next_poll_not_before_ms = transport.now_ms();
        for poll_index in 1..=self.limits.maximum_polls {
            if transport.now_ms() > deadline_at_ms {
                return Err(AdapterError::DeadlineExceeded);
            }
            transport
                .wait_until_ms(next_poll_not_before_ms, deadline_at_ms)
                .map_err(AdapterError::Transport)?;
            if transport.now_ms() > deadline_at_ms {
                return Err(AdapterError::DeadlineExceeded);
            }
            let result_transport_id = next_transport_id(
                request_ids,
                &request.owner_action_request_id,
                &mut used_transport_ids,
            )?;
            let result_response = self.send_signed(
                transport,
                pairing_key,
                &request.endpoint,
                HttpMethod::Get,
                &result_target,
                Vec::new(),
                &result_transport_id,
                deadline_at_ms,
            )?;
            let owner_result: OwnerResult = serde_json::from_slice(&result_response.body)
                .map_err(|_| AdapterError::InvalidJson)?;
            if owner_result.request_id != request.owner_action_request_id {
                return Err(AdapterError::OwnerResultMismatch);
            }
            if !owner_result.accepted {
                return Err(AdapterError::OwnerRejected);
            }
            if !owner_result.completed {
                let interval = i64::try_from(self.limits.poll_interval_ms)
                    .map_err(|_| AdapterError::InvalidLimits)?;
                next_poll_not_before_ms = result_response
                    .received_at_ms
                    .checked_add(interval)
                    .ok_or(AdapterError::InvalidLimits)?;
                continue;
            }
            if owner_result.schema.as_deref() != Some(KIOSK_CLI_RESULT_SCHEMA)
                || owner_result.command.as_deref() != Some(KIOSK_SHOW_CONTROLS_COMMAND)
                || owner_result.recorded_at_ms.is_none()
                || owner_result
                    .state
                    .as_ref()
                    .is_none_or(|state| !state.controls_open)
            {
                return Err(AdapterError::OwnerResultMismatch);
            }
            let owner_recorded_at_ms = owner_result
                .recorded_at_ms
                .ok_or(AdapterError::OwnerResultMismatch)?;
            let response_signature = header(&result_response.headers, HEADER_SIGNATURE)
                .ok_or(AdapterError::MissingResponseHeader(HEADER_SIGNATURE))?
                .to_owned();
            let raw_sha = sha256_hex(&result_response.body);
            let wrapped_at_ms = result_response.received_at_ms;
            let effective_receipt = KioskEffectiveReceipt {
                schema: "rusty.fleet.kiosk_effective_receipt.v1".to_owned(),
                receipt_id: request.receipt_id.clone(),
                operation_id: request.operation_id.clone(),
                device_id: request.device_id.clone(),
                identity_revision: request.identity_revision,
                owner_contract,
                owner_action_request_id: request.owner_action_request_id.clone(),
                owner_result_transport_request_id: result_transport_id.clone(),
                owner_command: KIOSK_SHOW_CONTROLS_COMMAND.to_owned(),
                response_status: result_response.status,
                response_content_sha256: raw_sha.clone(),
                response_signature,
                response_auth_verified: true,
                owner_result_schema: KIOSK_CLI_RESULT_SCHEMA.to_owned(),
                owner_accepted: owner_result.accepted,
                owner_completed: owner_result.completed,
                owner_recorded_at_ms,
                controls_open: true,
                wrapped_at_ms,
            };
            if effective_receipt.validate().is_err() {
                return Err(AdapterError::EffectiveReceiptInvalid);
            }
            return Ok(KioskShowControlsOutcome {
                effective_receipt,
                owner_receipt: RawOwnerReceiptEvidence {
                    raw_receipt_sha256: raw_sha,
                    raw_receipt: result_response.body,
                    host_received_at_ms: wrapped_at_ms,
                    result_transport_request_id: result_transport_id,
                },
                status_transport_request_id: status_transport_id,
                invoke_transport_request_id: invoke_transport_id,
                poll_count: poll_index,
            });
        }
        Err(AdapterError::PollLimitExceeded)
    }

    pub fn invoke_show_controls<T, I>(
        &self,
        request: &KioskShowControlsRequest,
        pairing_key: &str,
        transport: &mut T,
        request_ids: &mut I,
    ) -> Result<KioskShowControlsDispatch, AdapterError>
    where
        T: KioskTransport,
        I: TransportRequestIdSource,
    {
        request.validate()?;
        if pairing_key.is_empty() {
            return Err(AdapterError::InvalidRequest);
        }
        let started_at_ms = transport.now_ms();
        let deadline_delta = i64::try_from(self.limits.operation_deadline_ms)
            .map_err(|_| AdapterError::InvalidLimits)?;
        let deadline_at_ms = started_at_ms
            .checked_add(deadline_delta)
            .ok_or(AdapterError::InvalidLimits)?;
        let mut used_transport_ids = BTreeSet::new();
        let status_transport_request_id = next_transport_id(
            request_ids,
            &request.owner_action_request_id,
            &mut used_transport_ids,
        )?;
        let status_response = self.send_signed(
            transport,
            pairing_key,
            &request.endpoint,
            HttpMethod::Get,
            STATUS_TARGET,
            Vec::new(),
            &status_transport_request_id,
            deadline_at_ms,
        )?;
        let status: OwnerStatus =
            serde_json::from_slice(&status_response.body).map_err(|_| AdapterError::InvalidJson)?;
        if !status.accepted || status.schema != KIOSK_DIRECT_OPERATOR_SCHEMA {
            return Err(AdapterError::OwnerContractMismatch);
        }

        let invoke_transport_request_id = next_transport_id(
            request_ids,
            &request.owner_action_request_id,
            &mut used_transport_ids,
        )?;
        let invoke_body = serde_json::to_vec(&OwnerInvoke {
            request_id: &request.owner_action_request_id,
            command: KIOSK_SHOW_CONTROLS_COMMAND,
        })
        .map_err(|_| AdapterError::InvalidJson)?;
        if invoke_body.len() > MAX_INVOKE_BODY_BYTES {
            return Err(AdapterError::InvalidRequest);
        }
        let invoke_response = self.send_signed(
            transport,
            pairing_key,
            &request.endpoint,
            HttpMethod::Post,
            INVOKE_TARGET,
            invoke_body,
            &invoke_transport_request_id,
            deadline_at_ms,
        )?;
        let invoke: OwnerInvokeResponse =
            serde_json::from_slice(&invoke_response.body).map_err(|_| AdapterError::InvalidJson)?;
        if !invoke.accepted
            || invoke.completed
            || invoke.request_id != request.owner_action_request_id
        {
            return Err(AdapterError::OwnerInvokeMismatch);
        }
        Ok(KioskShowControlsDispatch {
            deadline_at_ms,
            status_transport_request_id,
            invoke_transport_request_id,
        })
    }

    pub fn poll_show_controls_result<T, I>(
        &self,
        request: &KioskShowControlsRequest,
        pairing_key: &str,
        deadline_at_ms: i64,
        transport: &mut T,
        request_ids: &mut I,
    ) -> Result<KioskShowControlsPollOutcome, AdapterError>
    where
        T: KioskTransport,
        I: TransportRequestIdSource,
    {
        request.validate()?;
        if pairing_key.is_empty() {
            return Err(AdapterError::InvalidRequest);
        }
        let now_ms = transport.now_ms();
        let maximum_window_ms = i64::from(KIOSK_DIRECT_OPERATOR_MAX_CLOCK_SKEW_SECONDS) * 1_000;
        if deadline_at_ms <= now_ms
            || deadline_at_ms
                .checked_sub(now_ms)
                .is_none_or(|remaining| remaining > maximum_window_ms)
        {
            return Err(AdapterError::DeadlineExceeded);
        }
        let owner_contract = KioskOwnerContractBinding::show_controls_v1();
        if owner_contract.validate().is_err() {
            return Err(AdapterError::OwnerContractMismatch);
        }
        let result_target = format!(
            "{RESULT_TARGET}?request_id={}",
            request.owner_action_request_id
        );
        let mut used_transport_ids = BTreeSet::new();
        let mut next_poll_not_before_ms = now_ms;
        for poll_index in 1..=self.limits.maximum_polls {
            if transport.now_ms() > deadline_at_ms {
                return Err(AdapterError::DeadlineExceeded);
            }
            transport
                .wait_until_ms(next_poll_not_before_ms, deadline_at_ms)
                .map_err(AdapterError::Transport)?;
            if transport.now_ms() > deadline_at_ms {
                return Err(AdapterError::DeadlineExceeded);
            }
            let result_transport_id = next_transport_id(
                request_ids,
                &request.owner_action_request_id,
                &mut used_transport_ids,
            )?;
            let result_response = self.send_signed(
                transport,
                pairing_key,
                &request.endpoint,
                HttpMethod::Get,
                &result_target,
                Vec::new(),
                &result_transport_id,
                deadline_at_ms,
            )?;
            let owner_result: OwnerResult = serde_json::from_slice(&result_response.body)
                .map_err(|_| AdapterError::InvalidJson)?;
            if owner_result.request_id != request.owner_action_request_id {
                return Err(AdapterError::OwnerResultMismatch);
            }
            if !owner_result.accepted {
                return Err(AdapterError::OwnerRejected);
            }
            if !owner_result.completed {
                let interval = i64::try_from(self.limits.poll_interval_ms)
                    .map_err(|_| AdapterError::InvalidLimits)?;
                next_poll_not_before_ms = result_response
                    .received_at_ms
                    .checked_add(interval)
                    .ok_or(AdapterError::InvalidLimits)?;
                continue;
            }
            if owner_result.schema.as_deref() != Some(KIOSK_CLI_RESULT_SCHEMA)
                || owner_result.command.as_deref() != Some(KIOSK_SHOW_CONTROLS_COMMAND)
                || owner_result.recorded_at_ms.is_none()
                || owner_result
                    .state
                    .as_ref()
                    .is_none_or(|state| !state.controls_open)
            {
                return Err(AdapterError::OwnerResultMismatch);
            }
            let owner_recorded_at_ms = owner_result
                .recorded_at_ms
                .ok_or(AdapterError::OwnerResultMismatch)?;
            let response_signature = header(&result_response.headers, HEADER_SIGNATURE)
                .ok_or(AdapterError::MissingResponseHeader(HEADER_SIGNATURE))?
                .to_owned();
            let raw_sha = sha256_hex(&result_response.body);
            let wrapped_at_ms = result_response.received_at_ms;
            let effective_receipt = KioskEffectiveReceipt {
                schema: "rusty.fleet.kiosk_effective_receipt.v1".to_owned(),
                receipt_id: request.receipt_id.clone(),
                operation_id: request.operation_id.clone(),
                device_id: request.device_id.clone(),
                identity_revision: request.identity_revision,
                owner_contract,
                owner_action_request_id: request.owner_action_request_id.clone(),
                owner_result_transport_request_id: result_transport_id.clone(),
                owner_command: KIOSK_SHOW_CONTROLS_COMMAND.to_owned(),
                response_status: result_response.status,
                response_content_sha256: raw_sha.clone(),
                response_signature,
                response_auth_verified: true,
                owner_result_schema: KIOSK_CLI_RESULT_SCHEMA.to_owned(),
                owner_accepted: owner_result.accepted,
                owner_completed: owner_result.completed,
                owner_recorded_at_ms,
                controls_open: true,
                wrapped_at_ms,
            };
            if effective_receipt.validate().is_err() {
                return Err(AdapterError::EffectiveReceiptInvalid);
            }
            return Ok(KioskShowControlsPollOutcome {
                effective_receipt,
                owner_receipt: RawOwnerReceiptEvidence {
                    raw_receipt_sha256: raw_sha,
                    raw_receipt: result_response.body,
                    host_received_at_ms: wrapped_at_ms,
                    result_transport_request_id: result_transport_id,
                },
                poll_count: poll_index,
            });
        }
        Err(AdapterError::PollLimitExceeded)
    }

    pub fn resume_show_controls_result<T, I>(
        &self,
        request: &KioskShowControlsRequest,
        pairing_key: &str,
        deadline_at_ms: i64,
        transport: &mut T,
        request_ids: &mut I,
    ) -> Result<KioskShowControlsPollOutcome, AdapterError>
    where
        T: KioskTransport,
        I: TransportRequestIdSource,
    {
        self.poll_show_controls_result(request, pairing_key, deadline_at_ms, transport, request_ids)
    }

    #[allow(clippy::too_many_arguments)]
    fn send_signed<T: KioskTransport>(
        &self,
        transport: &mut T,
        pairing_key: &str,
        endpoint: &str,
        method: HttpMethod,
        request_target: &str,
        body: Vec<u8>,
        transport_request_id: &str,
        deadline_at_ms: i64,
    ) -> Result<KioskHttpResponse, AdapterError> {
        let sent_at_ms = transport.now_ms();
        if sent_at_ms > deadline_at_ms {
            return Err(AdapterError::DeadlineExceeded);
        }
        let timestamp_seconds = sent_at_ms.div_euclid(1_000);
        let content_sha256 = sha256_hex(&body);
        let signature = sign_request(
            pairing_key,
            method.as_str(),
            request_target,
            transport_request_id,
            timestamp_seconds,
            &content_sha256,
        )?;
        let headers = BTreeMap::from([
            (
                HEADER_REQUEST_ID.to_owned(),
                transport_request_id.to_owned(),
            ),
            (HEADER_TIMESTAMP.to_owned(), timestamp_seconds.to_string()),
            (HEADER_CONTENT_SHA256.to_owned(), content_sha256),
            (HEADER_SIGNATURE.to_owned(), signature),
        ]);
        let response = transport
            .send(KioskHttpRequest {
                method,
                url: format!("{endpoint}{request_target}"),
                request_target: request_target.to_owned(),
                headers,
                body,
                redirect_policy: RedirectPolicy::Deny,
                maximum_response_bytes: self.limits.maximum_response_bytes,
                timeout_ms: self.limits.request_timeout_ms,
                deadline_at_ms,
            })
            .map_err(AdapterError::Transport)?;
        if response.redirected || (300..400).contains(&response.status) {
            return Err(AdapterError::RedirectRejected);
        }
        if response.received_at_ms > deadline_at_ms {
            return Err(AdapterError::DeadlineExceeded);
        }
        if response.body.len() > self.limits.maximum_response_bytes {
            return Err(AdapterError::ResponseTooLarge);
        }
        verify_signed_response(pairing_key, transport_request_id, &response)?;
        if response.status != 200 {
            return Err(AdapterError::UnexpectedHttpStatus(response.status));
        }
        Ok(response)
    }
}

#[derive(Serialize)]
struct OwnerInvoke<'a> {
    request_id: &'a str,
    command: &'a str,
}

#[derive(Deserialize)]
struct OwnerStatus {
    schema: String,
    accepted: bool,
}

#[derive(Deserialize)]
struct OwnerInvokeResponse {
    request_id: String,
    accepted: bool,
    completed: bool,
}

#[derive(Deserialize)]
struct OwnerResult {
    schema: Option<String>,
    request_id: String,
    command: Option<String>,
    accepted: bool,
    completed: bool,
    recorded_at_ms: Option<i64>,
    state: Option<OwnerResultState>,
}

#[derive(Deserialize)]
struct OwnerResultState {
    controls_open: bool,
}

fn next_transport_id<I: TransportRequestIdSource>(
    source: &mut I,
    owner_action_request_id: &str,
    used: &mut BTreeSet<String>,
) -> Result<String, AdapterError> {
    let request_id = source
        .next_transport_request_id()
        .map_err(AdapterError::Transport)?;
    if !is_request_id(&request_id) {
        return Err(AdapterError::InvalidTransportRequestId);
    }
    if request_id == owner_action_request_id || !used.insert(request_id.clone()) {
        return Err(AdapterError::DuplicateTransportRequestId);
    }
    Ok(request_id)
}

pub fn validate_kiosk_endpoint(endpoint: &str) -> Result<(), AdapterError> {
    if endpoint.len() > 255
        || !endpoint.starts_with("http://")
        || endpoint
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(AdapterError::InvalidEndpoint);
    }
    let authority = &endpoint["http://".len()..];
    let expected_port = format!(":{KIOSK_DIRECT_OPERATOR_PORT}");
    if authority.is_empty()
        || authority.contains(['/', '?', '#', '@'])
        || !authority.ends_with(&expected_port)
    {
        return Err(AdapterError::InvalidEndpoint);
    }
    let host = &authority[..authority.len() - expected_port.len()];
    if host.is_empty()
        || !host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
        })
    {
        return Err(AdapterError::InvalidEndpoint);
    }
    Ok(())
}

fn is_request_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn header<'a>(headers: &'a BTreeMap<String, String>, expected: &str) -> Option<&'a str> {
    headers.iter().find_map(|(name, value)| {
        name.eq_ignore_ascii_case(expected)
            .then_some(value.as_str())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sign_request(
    pairing_key: &str,
    method: &str,
    request_target: &str,
    request_id: &str,
    timestamp_seconds: i64,
    content_sha256: &str,
) -> Result<String, AdapterError> {
    if pairing_key.is_empty()
        || !is_request_id(request_id)
        || !is_lower_hex_sha256(content_sha256)
        || request_target.is_empty()
        || request_target.contains(['\r', '\n'])
    {
        return Err(AdapterError::InvalidRequest);
    }
    let canonical = format!(
        "{}\n{request_target}\n{request_id}\n{timestamp_seconds}\n{content_sha256}",
        method.to_ascii_uppercase()
    );
    Ok(hex::encode(hmac_sha256(
        pairing_key.as_bytes(),
        canonical.as_bytes(),
    )))
}

pub fn sign_response(
    pairing_key: &str,
    request_id: &str,
    status: u16,
    content_sha256: &str,
) -> Result<String, AdapterError> {
    if pairing_key.is_empty() || !is_request_id(request_id) || !is_lower_hex_sha256(content_sha256)
    {
        return Err(AdapterError::InvalidRequest);
    }
    let canonical = format!("RESPONSE\n{request_id}\n{status}\n{content_sha256}");
    Ok(hex::encode(hmac_sha256(
        pairing_key.as_bytes(),
        canonical.as_bytes(),
    )))
}

pub fn verify_signed_response(
    pairing_key: &str,
    expected_request_id: &str,
    response: &KioskHttpResponse,
) -> Result<(), AdapterError> {
    let response_request_id = header(&response.headers, HEADER_REQUEST_ID)
        .ok_or(AdapterError::MissingResponseHeader(HEADER_REQUEST_ID))?;
    let declared_sha = header(&response.headers, HEADER_CONTENT_SHA256)
        .ok_or(AdapterError::MissingResponseHeader(HEADER_CONTENT_SHA256))?;
    let signature = header(&response.headers, HEADER_SIGNATURE)
        .ok_or(AdapterError::MissingResponseHeader(HEADER_SIGNATURE))?;
    if response_request_id != expected_request_id {
        return Err(AdapterError::ResponseRequestIdMismatch);
    }
    let actual_sha = sha256_hex(&response.body);
    if !constant_time_equal(declared_sha.as_bytes(), actual_sha.as_bytes()) {
        return Err(AdapterError::ResponseDigestMismatch);
    }
    let expected_signature = sign_response(
        pairing_key,
        expected_request_id,
        response.status,
        &actual_sha,
    )?;
    if !constant_time_equal(signature.as_bytes(), expected_signature.as_bytes()) {
        return Err(AdapterError::ResponseSignatureMismatch);
    }
    Ok(())
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_BYTES: usize = 64;
    let mut normalized_key = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        normalized_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }
    let mut inner_key = [0x36_u8; BLOCK_BYTES];
    let mut outer_key = [0x5c_u8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_key[index] ^= normalized_key[index];
        outer_key[index] ^= normalized_key[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_key);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_key);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
