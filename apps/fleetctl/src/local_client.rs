// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use fleet_contracts::{
    AuthenticatedPackageUpdaterAcknowledgement, AuthenticatedPackageUpdaterReceipt,
    OperationExecuteRequest, OperationPreviewRequest, PackageInstallReleaseExecuteRequest,
    PackageInstallReleasePreviewRequest, PackageUpdaterClaimRequest, QuestAwakeExecuteRequest,
    QuestAwakePreviewRequest, QuestWifiAdbExecuteRequest, QuestWifiAdbPreviewRequest,
};

use crate::{CliFailure, FleetOperationClient};

const DEFAULT_HUB_URL: &str = "http://127.0.0.1:8741";
const HUB_URL_ENV: &str = "RUSTY_FLEET_HUB_URL";
const PACKAGE_OWNER_TOKEN_ENV: &str = "RUSTY_FLEET_PACKAGE_OWNER_TOKEN";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub struct LocalFleetOperationClient {
    authority: String,
    socket: SocketAddr,
}

impl LocalFleetOperationClient {
    pub fn from_env() -> Result<Self, CliFailure> {
        let base_url = std::env::var(HUB_URL_ENV).unwrap_or_else(|_| DEFAULT_HUB_URL.to_owned());
        Self::new(&base_url)
    }

    pub fn new(base_url: &str) -> Result<Self, CliFailure> {
        let authority = base_url
            .strip_prefix("http://")
            .filter(|authority| {
                !authority.is_empty()
                    && !authority.contains(['/', '?', '#', '@'])
                    && !authority.bytes().any(|byte| byte.is_ascii_whitespace())
            })
            .ok_or_else(|| {
                CliFailure::new(
                    "invalid_hub_url",
                    "Fleet Hub URL must be a plain loopback http://IP:PORT authority",
                )
            })?;
        let socket = authority.parse::<SocketAddr>().map_err(|error| {
            CliFailure::new(
                "invalid_hub_url",
                format!("Fleet Hub URL must use a literal IP socket: {error}"),
            )
        })?;
        if !socket.ip().is_loopback() {
            return Err(CliFailure::new(
                "invalid_hub_url",
                "fleetctl operation commands are restricted to a loopback Fleet Hub",
            ));
        }
        Ok(Self {
            authority: authority.to_owned(),
            socket,
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request_with_token(method, path, body, None)
    }

    fn request_with_token(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        bearer_token: Option<&str>,
    ) -> Result<serde_json::Value, CliFailure> {
        let body = body.unwrap_or_default();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut stream = TcpStream::connect_timeout(&self.socket, REQUEST_TIMEOUT)
            .map_err(|error| transport_failure("connect", error))?;
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .map_err(|error| transport_failure("bound read", error))?;
        stream
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .map_err(|error| transport_failure("bound write", error))?;
        let mut wire = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.authority,
            body.len()
        )
        .into_bytes();
        if !body.is_empty() {
            wire.extend_from_slice(b"Content-Type: application/json\r\n");
        }
        if let Some(token) = bearer_token {
            if token.len() < 32
                || token.len() > 512
                || token.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(CliFailure::new(
                    "invalid_package_owner_token",
                    "package owner token must contain 32..512 non-control bytes",
                ));
            }
            wire.extend_from_slice(format!("Authorization: Bearer {token}\r\n").as_bytes());
        }
        wire.extend_from_slice(b"\r\n");
        wire.extend_from_slice(&body);
        write_until(&mut stream, &wire, deadline)?;

        let response = read_until(&mut stream, deadline)?;
        parse_response(&response)
    }
}

impl FleetOperationClient for LocalFleetOperationClient {
    fn preview_operation(
        &mut self,
        request: &OperationPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            "/fleet/v1/operations/preview",
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn execute_operation(
        &mut self,
        request: &OperationExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            &format!(
                "/fleet/v1/operations/{}/execute",
                encode_path_segment(&request.operation_id)
            ),
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn get_operation(&mut self, operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "GET",
            &format!("/fleet/v1/operations/{}", encode_path_segment(operation_id)),
            None,
        )
    }

    fn preview_package_install_release(
        &mut self,
        request: &PackageInstallReleasePreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            "/fleet/v1/package-install-releases/preview",
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn execute_package_install_release(
        &mut self,
        request: &PackageInstallReleaseExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            &format!(
                "/fleet/v1/package-install-releases/{}/execute",
                encode_path_segment(&request.operation_id)
            ),
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn get_package_install_release(
        &mut self,
        operation_id: &str,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "GET",
            &format!(
                "/fleet/v1/package-install-releases/{}",
                encode_path_segment(operation_id)
            ),
            None,
        )
    }

    fn preview_quest_awake(
        &mut self,
        request: &QuestAwakePreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            "/fleet/v1/quest-awake/preview",
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn execute_quest_awake(
        &mut self,
        request: &QuestAwakeExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            &format!(
                "/fleet/v1/quest-awake/{}/execute",
                encode_path_segment(&request.operation_id)
            ),
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn get_quest_awake(&mut self, operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "GET",
            &format!(
                "/fleet/v1/quest-awake/{}",
                encode_path_segment(operation_id)
            ),
            None,
        )
    }

    fn preview_quest_wifi_adb(
        &mut self,
        request: &QuestWifiAdbPreviewRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            "/fleet/v1/quest-wifi-adb/preview",
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn execute_quest_wifi_adb(
        &mut self,
        request: &QuestWifiAdbExecuteRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "POST",
            &format!(
                "/fleet/v1/quest-wifi-adb/{}/execute",
                encode_path_segment(&request.operation_id)
            ),
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
        )
    }

    fn get_quest_wifi_adb(&mut self, operation_id: &str) -> Result<serde_json::Value, CliFailure> {
        self.request(
            "GET",
            &format!(
                "/fleet/v1/quest-wifi-adb/{}",
                encode_path_segment(operation_id)
            ),
            None,
        )
    }

    fn claim_package_updater(
        &mut self,
        request: &PackageUpdaterClaimRequest,
    ) -> Result<serde_json::Value, CliFailure> {
        let token = std::env::var(PACKAGE_OWNER_TOKEN_ENV).map_err(|_| {
            CliFailure::new(
                "package_owner_token_required",
                "set RUSTY_FLEET_PACKAGE_OWNER_TOKEN for owner ingress",
            )
        })?;
        self.request_with_token(
            "POST",
            "/fleet/v1/package-updater/claims",
            Some(serde_json::to_vec(request).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
            Some(&token),
        )
    }

    fn peek_package_updater_offer(&mut self) -> Result<serde_json::Value, CliFailure> {
        let token = std::env::var(PACKAGE_OWNER_TOKEN_ENV).map_err(|_| {
            CliFailure::new(
                "package_owner_token_required",
                "set RUSTY_FLEET_PACKAGE_OWNER_TOKEN for owner ingress",
            )
        })?;
        self.request_with_token(
            "GET",
            "/fleet/v1/package-updater/offers",
            None,
            Some(&token),
        )
    }

    fn submit_package_updater_acknowledgement(
        &mut self,
        operation_id: &str,
        submission: &AuthenticatedPackageUpdaterAcknowledgement,
    ) -> Result<serde_json::Value, CliFailure> {
        let token = std::env::var(PACKAGE_OWNER_TOKEN_ENV).map_err(|_| {
            CliFailure::new(
                "package_owner_token_required",
                "set RUSTY_FLEET_PACKAGE_OWNER_TOKEN for owner ingress",
            )
        })?;
        self.request_with_token(
            "POST",
            &format!(
                "/fleet/v1/package-install-releases/{}/acknowledgements",
                encode_path_segment(operation_id)
            ),
            Some(serde_json::to_vec(submission).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
            Some(&token),
        )
    }

    fn submit_package_updater_receipt(
        &mut self,
        operation_id: &str,
        submission: &AuthenticatedPackageUpdaterReceipt,
    ) -> Result<serde_json::Value, CliFailure> {
        let token = std::env::var(PACKAGE_OWNER_TOKEN_ENV).map_err(|_| {
            CliFailure::new(
                "package_owner_token_required",
                "set RUSTY_FLEET_PACKAGE_OWNER_TOKEN for owner ingress",
            )
        })?;
        self.request_with_token(
            "POST",
            &format!(
                "/fleet/v1/package-install-releases/{}/receipts",
                encode_path_segment(operation_id)
            ),
            Some(serde_json::to_vec(submission).map_err(|error| {
                CliFailure::new("request_serialization_failed", error.to_string())
            })?),
            Some(&token),
        )
    }
}

fn write_until(stream: &mut TcpStream, bytes: &[u8], deadline: Instant) -> Result<(), CliFailure> {
    let mut written = 0;
    while written < bytes.len() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|error| transport_failure("refresh write bound", error))?;
        let count = stream
            .write(&bytes[written..])
            .map_err(|error| transport_failure("write request", error))?;
        if count == 0 {
            return Err(CliFailure::new(
                "hub_transport_failed",
                "Fleet Hub closed while fleetctl was writing",
            ));
        }
        written = written.saturating_add(count);
    }
    stream
        .flush()
        .map_err(|error| transport_failure("flush request", error))
}

fn read_until(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, CliFailure> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|error| transport_failure("refresh read bound", error))?;
        let count = stream
            .read(&mut chunk)
            .map_err(|error| transport_failure("read response", error))?;
        if count == 0 {
            break;
        }
        if response.len().saturating_add(count) > MAX_RESPONSE_BYTES + 32 * 1024 {
            return Err(CliFailure::new(
                "hub_response_too_large",
                "Fleet Hub response exceeded 16 MiB",
            ));
        }
        response.extend_from_slice(&chunk[..count]);
        if let Some(expected) = expected_wire_length(&response)? {
            if response.len() > expected {
                return Err(CliFailure::new(
                    "invalid_hub_response",
                    "Fleet Hub response exceeded Content-Length",
                ));
            }
            if response.len() == expected {
                break;
            }
        }
    }
    Ok(response)
}

fn expected_wire_length(response: &[u8]) -> Result<Option<usize>, CliFailure> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        if response.len() > 32 * 1024 {
            return Err(CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub response header exceeded 32 KiB",
            ));
        }
        return Ok(None);
    };
    let (_, content_length) = parse_headers(&response[..header_end])?;
    Ok(Some(
        header_end
            .checked_add(4)
            .and_then(|value| value.checked_add(content_length))
            .ok_or_else(|| {
                CliFailure::new(
                    "invalid_hub_response",
                    "Fleet Hub response length overflowed",
                )
            })?,
    ))
}

fn parse_response(response: &[u8]) -> Result<serde_json::Value, CliFailure> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub response omitted a complete HTTP header",
            )
        })?;
    let (status, content_length) = parse_headers(&response[..header_end])?;
    let body_start = header_end + 4;
    if response.len() != body_start.saturating_add(content_length) {
        return Err(CliFailure::new(
            "invalid_hub_response",
            "Fleet Hub response differs from Content-Length",
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response[body_start..]).map_err(|error| {
            CliFailure::new(
                "invalid_hub_response",
                format!("Fleet Hub response is not valid JSON: {error}"),
            )
        })?;
    if status == 200 {
        return Ok(value);
    }
    let code = value
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("hub_request_failed");
    let message = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map_or_else(
            || format!("Fleet Hub returned HTTP {status}"),
            str::to_owned,
        );
    Err(CliFailure::new(code, message))
}

fn parse_headers(header: &[u8]) -> Result<(u16, usize), CliFailure> {
    let text = std::str::from_utf8(header).map_err(|_| {
        CliFailure::new(
            "invalid_hub_response",
            "Fleet Hub response header is not UTF-8",
        )
    })?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut status_parts = status_line.split_whitespace();
    let protocol = status_parts.next().unwrap_or_default();
    let status = status_parts
        .next()
        .unwrap_or_default()
        .parse::<u16>()
        .map_err(|_| {
            CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub response status is invalid",
            )
        })?;
    if protocol != "HTTP/1.1" && protocol != "HTTP/1.0" {
        return Err(CliFailure::new(
            "invalid_hub_response",
            "Fleet Hub response protocol is unsupported",
        ));
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| {
            CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub response header line is malformed",
            )
        })?;
        let name = name.trim().to_ascii_lowercase();
        if headers.insert(name, value.trim().to_owned()).is_some() {
            return Err(CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub response repeated a header",
            ));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(CliFailure::new(
            "invalid_hub_response",
            "fleetctl does not accept chunked Fleet Hub responses",
        ));
    }
    let content_length = headers
        .get("content-length")
        .ok_or_else(|| {
            CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub response omitted Content-Length",
            )
        })?
        .parse::<usize>()
        .map_err(|_| {
            CliFailure::new(
                "invalid_hub_response",
                "Fleet Hub Content-Length is invalid",
            )
        })?;
    if content_length > MAX_RESPONSE_BYTES {
        return Err(CliFailure::new(
            "hub_response_too_large",
            "Fleet Hub response exceeded 16 MiB",
        ));
    }
    Ok((status, content_length))
}

fn remaining(deadline: Instant) -> Result<Duration, CliFailure> {
    deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| {
            CliFailure::new(
                "hub_transport_timeout",
                "Fleet Hub request exceeded five seconds",
            )
        })
}

fn transport_failure(action: &str, error: std::io::Error) -> CliFailure {
    CliFailure::new(
        "hub_transport_failed",
        format!("could not {action}: {error}"),
    )
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{LocalFleetOperationClient, encode_path_segment, parse_response};

    #[test]
    fn local_client_is_loopback_only_and_encodes_operation_ids() {
        assert!(LocalFleetOperationClient::new("http://127.0.0.1:8741").is_ok());
        assert!(LocalFleetOperationClient::new("http://[::1]:8741").is_ok());
        assert!(LocalFleetOperationClient::new("http://192.0.2.1:8741").is_err());
        assert!(LocalFleetOperationClient::new("https://127.0.0.1:8741").is_err());
        assert_eq!(encode_path_segment("operation/a b"), "operation%2Fa%20b");
    }

    #[test]
    fn local_client_requires_fixed_length_json_responses() {
        let body = br#"{"schema":"test.v1"}"#;
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n",
            body.len()
        );
        let mut response = wire.into_bytes();
        response.extend_from_slice(body);
        assert_eq!(
            parse_response(&response).expect("valid response")["schema"],
            "test.v1"
        );
        assert!(
            parse_response(b"HTTP/1.1 200 OK\r\n\r\n{}").is_err(),
            "missing Content-Length must fail"
        );
    }
}
