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
