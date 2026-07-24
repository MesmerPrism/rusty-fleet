// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CapabilitySnapshot, ContractViolation, StatusCondition, StreamDescriptor, ValidateContract,
    finish, require_nonempty,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub identity_revision: u64,
    pub display_name: String,
    pub model: String,
    pub hardware_class: String,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ValidateContract for DeviceIdentity {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.device_id, "identity.device_id");
        require_nonempty(&mut failures, &self.display_name, "identity.display_name");
        require_nonempty(&mut failures, &self.model, "identity.model");
        require_nonempty(
            &mut failures,
            &self.hardware_class,
            "identity.hardware_class",
        );
        if self.identity_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "identity.identity_revision",
                "identity revision must be greater than zero",
            ));
        }
        if self
            .tags
            .iter()
            .any(|(key, value)| key.trim().is_empty() || value.trim().is_empty())
        {
            failures.push(ContractViolation::new(
                "invalid_tag",
                "identity.tags",
                "tag keys and values must not be empty",
            ));
        }
        if self.tags.len() > 128 || self.extensions.len() > 64 {
            failures.push(ContractViolation::new(
                "identity_too_large",
                "identity",
                "identity supports at most 128 tags and 64 extension fields",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KioskState {
    Active,
    Inactive,
    Mismatch,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactProvenance {
    pub owner: String,
    pub adapter_id: String,
    pub observed_at_ms: i64,
    pub fresh_until_ms: i64,
}

impl ValidateContract for FactProvenance {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        require_nonempty(&mut failures, &self.owner, "owner");
        require_nonempty(&mut failures, &self.adapter_id, "adapter_id");
        if self.fresh_until_ms < self.observed_at_ms {
            failures.push(ContractViolation::new(
                "invalid_freshness",
                "fresh_until_ms",
                "fact freshness must not end before observation time",
            ));
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerObservation {
    pub battery_percent: u8,
    pub charging: bool,
    pub provenance: FactProvenance,
}

impl ValidateContract for PowerObservation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.battery_percent > 100 {
            failures.push(ContractViolation::new(
                "invalid_battery",
                "battery_percent",
                "battery percentage must be between 0 and 100",
            ));
        }
        if let Err(mut nested) = self.provenance.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationLifecycle {
    Foreground,
    Visible,
    Background,
    Stopped,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundState {
    Foreground,
    Background,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundAuthority {
    SelfReport,
    ParticipatingApp,
    PlatformLimited,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationObservation {
    pub package_name: Option<String>,
    pub lifecycle: ApplicationLifecycle,
    pub foreground_state: ForegroundState,
    pub foreground_authority: ForegroundAuthority,
    pub provenance: FactProvenance,
}

impl ValidateContract for ApplicationObservation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self
            .package_name
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            failures.push(ContractViolation::new(
                "invalid_package",
                "package_name",
                "application package must be absent or nonempty",
            ));
        }
        if self.foreground_authority == ForegroundAuthority::Unavailable
            && self.foreground_state != ForegroundState::Unknown
        {
            failures.push(ContractViolation::new(
                "unsupported_foreground_claim",
                "foreground_state",
                "unavailable foreground authority may report only unknown state",
            ));
        }
        if matches!(
            self.foreground_authority,
            ForegroundAuthority::SelfReport | ForegroundAuthority::ParticipatingApp
        ) && self.package_name.is_none()
        {
            failures.push(ContractViolation::new(
                "missing_application_identity",
                "package_name",
                "self-reported and participating-app evidence must name its package",
            ));
        }
        if let Err(mut nested) = self.provenance.validate() {
            failures.append(&mut nested);
        }
        finish(failures)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceObservation {
    pub schema: String,
    pub identity: DeviceIdentity,
    pub source_epoch: String,
    pub source_revision: u64,
    pub source_time_ms: i64,
    pub received_time_ms: i64,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub foreground_app: Option<String>,
    #[serde(default)]
    pub agent: Option<ApplicationObservation>,
    #[serde(default)]
    pub power: Option<PowerObservation>,
    #[serde(default)]
    pub application: Option<ApplicationObservation>,
    pub kiosk_state: KioskState,
    #[serde(default)]
    pub conditions: Vec<StatusCondition>,
    pub capabilities: CapabilitySnapshot,
    #[serde(default)]
    pub streams: Vec<StreamDescriptor>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ValidateContract for DeviceObservation {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.device_observation.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.device_observation.v1",
            ));
        }
        if let Err(mut nested) = self.identity.validate() {
            failures.append(&mut nested);
        }
        require_nonempty(&mut failures, &self.source_epoch, "source_epoch");
        if self.source_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "source_revision",
                "source revision must be greater than zero",
            ));
        }
        if let Some(percent) = self.battery_percent
            && percent > 100
        {
            failures.push(ContractViolation::new(
                "invalid_battery",
                "battery_percent",
                "battery percentage must be between 0 and 100",
            ));
        }
        if let Some(agent) = &self.agent {
            if let Err(nested) = agent.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("agent.{}", failure.path),
                    ..failure
                }));
            }
            if agent.foreground_authority != ForegroundAuthority::SelfReport {
                failures.push(ContractViolation::new(
                    "invalid_agent_authority",
                    "agent.foreground_authority",
                    "Fleet Agent lifecycle evidence must be self-reported",
                ));
            }
        }
        if let Some(power) = &self.power {
            if let Err(nested) = power.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("power.{}", failure.path),
                    ..failure
                }));
            }
            if self
                .battery_percent
                .is_some_and(|value| value != power.battery_percent)
                || self.charging.is_some_and(|value| value != power.charging)
            {
                failures.push(ContractViolation::new(
                    "power_projection_mismatch",
                    "power",
                    "legacy power fields must agree with the provenance-bearing power fact",
                ));
            }
        }
        if let Some(application) = &self.application {
            if let Err(nested) = application.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("application.{}", failure.path),
                    ..failure
                }));
            }
            if self
                .foreground_app
                .as_ref()
                .is_some_and(|value| application.package_name.as_ref() != Some(value))
            {
                failures.push(ContractViolation::new(
                    "application_projection_mismatch",
                    "application.package_name",
                    "legacy foreground application must agree with the provenance-bearing application fact",
                ));
            }
        }
        if self.conditions.len() > 64
            || self.streams.len() > 32
            || self.capabilities.capabilities.len() > 128
            || self.extensions.len() > 64
        {
            failures.push(ContractViolation::new(
                "observation_too_large",
                "observation",
                "observation collection limits were exceeded",
            ));
        }
        if serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() > 2 * 1024 * 1024) {
            failures.push(ContractViolation::new(
                "observation_bytes_exceeded",
                "observation",
                "serialized observation exceeds the 2 MiB contract limit",
            ));
        }
        for (index, condition) in self.conditions.iter().enumerate() {
            if let Err(nested) = condition.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("conditions[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        if let Err(mut nested) = self.capabilities.validate() {
            failures.append(&mut nested);
        }
        for (index, stream) in self.streams.iter().enumerate() {
            if let Err(nested) = stream.validate() {
                failures.extend(nested.into_iter().map(|failure| ContractViolation {
                    path: format!("streams[{index}].{}", failure.path),
                    ..failure
                }));
            }
        }
        finish(failures)
    }
}
