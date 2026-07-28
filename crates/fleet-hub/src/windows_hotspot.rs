// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use fleet_contracts::{
    CommandLifecycle, ValidateContract, WINDOWS_HOTSPOT_ACTION_ID, WINDOWS_HOTSPOT_LEASE_SCHEMA,
    WINDOWS_HOTSPOT_OPERATION_SCHEMA, WINDOWS_HOTSPOT_OWNER,
    WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA, WINDOWS_HOTSPOT_RESOURCE_ID, WindowsHotspotAction,
    WindowsHotspotLease, WindowsHotspotOperation, WindowsHotspotOwnership, WindowsHotspotPreflight,
    WindowsHotspotPreview, WindowsHotspotPreviewRequest, WindowsHotspotProviderReceipt,
    WindowsHotspotProviderRequest, WindowsHotspotResult,
};
use serde::{Deserialize, Serialize};

use crate::{FleetHub, HubError, MAX_WINDOWS_HOTSPOT_OPERATIONS};

const PREVIEW_MAX_LIFETIME_MS: i64 = 5 * 60_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsHotspotPreviewPlan {
    pub operation_id: String,
    pub preview_id: String,
    pub lease_id: String,
    pub generation: String,
    pub request: WindowsHotspotPreviewRequest,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub provider_ready: bool,
}

impl FleetHub {
    pub fn preview_windows_hotspot(
        &mut self,
        plan: WindowsHotspotPreviewPlan,
    ) -> Result<WindowsHotspotOperation, HubError> {
        if plan.request.validate().is_err()
            || plan.operation_id.is_empty()
            || plan.preview_id.is_empty()
            || plan.lease_id.is_empty()
            || plan.generation.is_empty()
            || plan.created_at_ms < 0
            || plan.expires_at_ms <= plan.created_at_ms
            || plan.expires_at_ms - plan.created_at_ms > PREVIEW_MAX_LIFETIME_MS
        {
            return Err(HubError::new(
                "windows_hotspot_preview_plan_invalid",
                "Windows hotspot preview plan is invalid or exceeds five minutes",
            ));
        }
        if let Some(existing) = self.windows_hotspot_operations.get(&plan.operation_id) {
            return if existing.preview.preview_id == plan.preview_id
                && existing.preview.action == plan.request.action
            {
                Ok(existing.clone())
            } else {
                Err(HubError::new(
                    "windows_hotspot_operation_id_conflict",
                    "operation ID names a different immutable hotspot preview",
                ))
            };
        }
        let lease_available = !self.has_active_hotspot_lease(plan.created_at_ms, None);
        let observed = self.windows_hotspot_observation.clone();
        let ownership = observed
            .as_ref()
            .map_or(WindowsHotspotOwnership::None, |value| value.ownership);
        let active = observed.as_ref().is_some_and(|value| value.active);
        let action_eligible = match plan.request.action {
            WindowsHotspotAction::Status => true,
            WindowsHotspotAction::Start | WindowsHotspotAction::Ensure => {
                ownership != WindowsHotspotOwnership::External
            }
            WindowsHotspotAction::Stop => {
                active
                    && ownership == WindowsHotspotOwnership::Fleet
                    && observed
                        .as_ref()
                        .and_then(|value| value.generation.as_ref())
                        .is_some()
            }
        };
        let eligible = plan.provider_ready && lease_available && action_eligible;
        let reason_code = if !plan.provider_ready {
            "provider_unavailable"
        } else if !lease_available {
            "resource_leased"
        } else if ownership == WindowsHotspotOwnership::External {
            "external_hotspot_not_owned"
        } else if plan.request.action == WindowsHotspotAction::Stop && !action_eligible {
            "fleet_ownership_required"
        } else {
            "ready"
        };
        let operation = WindowsHotspotOperation {
            schema: WINDOWS_HOTSPOT_OPERATION_SCHEMA.to_owned(),
            operation_id: plan.operation_id.clone(),
            action_id: WINDOWS_HOTSPOT_ACTION_ID.to_owned(),
            lifecycle: if eligible {
                CommandLifecycle::Proposed
            } else {
                CommandLifecycle::Rejected
            },
            preview: WindowsHotspotPreview {
                schema: "rusty.fleet.windows_hotspot_preview.v1".to_owned(),
                preview_id: plan.preview_id,
                operation_id: plan.operation_id.clone(),
                action_id: WINDOWS_HOTSPOT_ACTION_ID.to_owned(),
                resource_id: WINDOWS_HOTSPOT_RESOURCE_ID.to_owned(),
                owner_id: WINDOWS_HOTSPOT_OWNER.to_owned(),
                action: plan.request.action,
                created_at_ms: plan.created_at_ms,
                expires_at_ms: plan.expires_at_ms,
                fleet_revision: self.result_revision,
                preflight: WindowsHotspotPreflight {
                    provider_ready: plan.provider_ready,
                    lease_available,
                    active,
                    ownership,
                    ownership_generation: observed.and_then(|value| value.generation),
                    eligible,
                    reason_code: reason_code.to_owned(),
                    message: if eligible {
                        "Host-scoped hotspot operation is ready for explicit confirmation"
                            .to_owned()
                    } else {
                        "Host-scoped hotspot operation is not eligible".to_owned()
                    },
                },
            },
            confirmed_at_ms: None,
            lease: Some(WindowsHotspotLease {
                schema: WINDOWS_HOTSPOT_LEASE_SCHEMA.to_owned(),
                lease_id: plan.lease_id,
                resource_id: WINDOWS_HOTSPOT_RESOURCE_ID.to_owned(),
                holder_operation_id: plan.operation_id.clone(),
                generation: plan.generation,
                issued_at_ms: plan.created_at_ms,
                expires_at_ms: plan.expires_at_ms,
            }),
            invocation: None,
            receipt: None,
            failure_code: (!eligible).then(|| reason_code.to_owned()),
            updated_at_ms: plan.created_at_ms,
        };
        operation
            .validate()
            .map_err(|_| HubError::new("windows_hotspot_operation_invalid", "operation invalid"))?;
        if self.windows_hotspot_operations.len() >= MAX_WINDOWS_HOTSPOT_OPERATIONS {
            return Err(HubError::new(
                "windows_hotspot_operation_limit_reached",
                "Windows hotspot operation retention limit reached",
            ));
        }
        self.windows_hotspot_operations
            .insert(plan.operation_id, operation.clone());
        Ok(operation)
    }

    pub fn confirm_windows_hotspot(
        &mut self,
        operation_id: &str,
        preview_id: &str,
        now_ms: i64,
    ) -> Result<WindowsHotspotOperation, HubError> {
        let existing = self.windows_hotspot_operation(operation_id)?;
        if existing.preview.preview_id != preview_id {
            return Err(HubError::new(
                "windows_hotspot_preview_conflict",
                "execute request does not bind the immutable preview",
            ));
        }
        if existing.lifecycle != CommandLifecycle::Proposed {
            return Ok(existing);
        }
        if now_ms > existing.preview.expires_at_ms {
            self.expire_windows_hotspot_operation(operation_id, now_ms)?;
            return Err(HubError::new(
                "windows_hotspot_preview_expired",
                "hotspot preview expired before confirmation",
            ));
        }
        if self.has_active_hotspot_lease(now_ms, Some(operation_id)) {
            return Err(HubError::new(
                "windows_hotspot_resource_leased",
                "another operation owns the Windows hotspot resource",
            ));
        }
        let operation = self
            .windows_hotspot_operations
            .get_mut(operation_id)
            .expect("operation was just read");
        operation.lifecycle = CommandLifecycle::Accepted;
        operation.confirmed_at_ms = Some(now_ms);
        operation.updated_at_ms = now_ms;
        Ok(operation.clone())
    }

    pub fn prepare_windows_hotspot_invocation(
        &mut self,
        operation_id: &str,
        request_id: String,
        now_ms: i64,
    ) -> Result<WindowsHotspotOperation, HubError> {
        let existing = self.windows_hotspot_operation(operation_id)?;
        if existing.lifecycle == CommandLifecycle::Dispatched && existing.invocation.is_some() {
            return Ok(existing);
        }
        if existing.lifecycle != CommandLifecycle::Accepted
            || now_ms > existing.preview.expires_at_ms
        {
            return Err(HubError::new(
                "windows_hotspot_dispatch_state_invalid",
                "only an unexpired confirmed operation can dispatch",
            ));
        }
        let _lease = existing.lease.as_ref().ok_or_else(|| {
            HubError::new(
                "windows_hotspot_lease_missing",
                "operation has no host lease",
            )
        })?;
        let current_generation = self
            .windows_hotspot_observation
            .as_ref()
            .and_then(|value| value.generation.clone());
        let ownership_generation = match existing.preview.action {
            WindowsHotspotAction::Start | WindowsHotspotAction::Status => None,
            WindowsHotspotAction::Ensure => {
                if current_generation != existing.preview.preflight.ownership_generation {
                    return Err(HubError::new(
                        "windows_hotspot_generation_changed",
                        "Fleet hotspot generation changed after preview",
                    ));
                }
                current_generation
            }
            WindowsHotspotAction::Stop => {
                let expected = current_generation.ok_or_else(|| {
                    HubError::new(
                        "windows_hotspot_ownership_lost",
                        "Fleet ownership disappeared after preview",
                    )
                })?;
                if expected
                    != existing
                        .preview
                        .preflight
                        .ownership_generation
                        .clone()
                        .unwrap_or_default()
                {
                    return Err(HubError::new(
                        "windows_hotspot_generation_changed",
                        "Fleet hotspot generation changed after preview",
                    ));
                }
                Some(expected)
            }
        };
        let invocation = WindowsHotspotProviderRequest {
            schema: WINDOWS_HOTSPOT_PROVIDER_REQUEST_SCHEMA.to_owned(),
            request_id,
            operation_id: existing.operation_id.clone(),
            action: existing.preview.action,
            expires_at_utc: unix_ms_to_rfc3339(existing.preview.expires_at_ms)?,
            timeout_ms: 30_000,
            ownership_generation,
        };
        invocation.validate().map_err(|_| {
            HubError::new(
                "windows_hotspot_invocation_invalid",
                "provider invocation invalid",
            )
        })?;
        let operation = self
            .windows_hotspot_operations
            .get_mut(operation_id)
            .expect("operation was just read");
        operation.invocation = Some(invocation);
        operation.lifecycle = CommandLifecycle::Dispatched;
        operation.updated_at_ms = now_ms;
        Ok(operation.clone())
    }

    pub fn apply_windows_hotspot_receipt(
        &mut self,
        receipt: WindowsHotspotProviderReceipt,
        now_ms: i64,
    ) -> Result<WindowsHotspotOperation, HubError> {
        receipt.validate().map_err(|_| {
            HubError::new(
                "windows_hotspot_receipt_invalid",
                "provider receipt is invalid",
            )
        })?;
        let existing = self.windows_hotspot_operation(&receipt.operation_id)?;
        if existing.lifecycle != CommandLifecycle::Dispatched || existing.receipt.is_some() {
            return Err(HubError::new(
                "windows_hotspot_receipt_replay",
                "only one receipt may settle a dispatched hotspot invocation",
            ));
        }
        let invocation = existing.invocation.as_ref().ok_or_else(|| {
            HubError::new(
                "windows_hotspot_invocation_missing",
                "operation has no invocation",
            )
        })?;
        if receipt.request_id != invocation.request_id || receipt.action != invocation.action {
            return Err(HubError::new(
                "windows_hotspot_receipt_binding_mismatch",
                "receipt does not bind the exact provider invocation",
            ));
        }
        if receipt.outcome == WindowsHotspotResult::Verified {
            let semantic_match = match receipt.action {
                WindowsHotspotAction::Status => true,
                WindowsHotspotAction::Start | WindowsHotspotAction::Ensure => {
                    receipt.operational_state == "On"
                        && receipt
                            .ownership_generation
                            .as_deref()
                            .is_some_and(|value| !value.is_empty())
                        && (receipt.action != WindowsHotspotAction::Ensure
                            || invocation.ownership_generation.is_none()
                            || receipt.ownership_generation == invocation.ownership_generation)
                }
                WindowsHotspotAction::Stop => {
                    receipt.operational_state == "Off"
                        && receipt.ownership_generation.is_none()
                        && invocation.ownership_generation.is_some()
                }
            };
            if !semantic_match {
                return Err(HubError::new(
                    "windows_hotspot_optimistic_success",
                    "verified receipt contradicts action and ownership readback",
                ));
            }
        }
        let prior = self.windows_hotspot_observation.clone();
        self.windows_hotspot_observation = match (
            receipt.outcome,
            receipt.action,
            receipt.operational_state.as_str(),
        ) {
            (
                WindowsHotspotResult::Verified,
                WindowsHotspotAction::Start | WindowsHotspotAction::Ensure,
                "On",
            ) => Some(WindowsHotspotObservedState {
                active: true,
                ownership: WindowsHotspotOwnership::Fleet,
                generation: receipt.ownership_generation.clone(),
                observed_at_ms: now_ms,
            }),
            (WindowsHotspotResult::Verified, WindowsHotspotAction::Status, "On")
                if prior.as_ref().is_some_and(|value| {
                    value.ownership == WindowsHotspotOwnership::Fleet
                        && receipt
                            .ownership_generation
                            .as_ref()
                            .is_none_or(|generation| Some(generation) == value.generation.as_ref())
                }) =>
            {
                prior
            }
            (WindowsHotspotResult::Verified, WindowsHotspotAction::Status, "On") => {
                Some(WindowsHotspotObservedState {
                    active: true,
                    ownership: WindowsHotspotOwnership::External,
                    generation: None,
                    observed_at_ms: now_ms,
                })
            }
            (WindowsHotspotResult::Verified, WindowsHotspotAction::Stop, "Off")
            | (WindowsHotspotResult::Verified, WindowsHotspotAction::Status, "Off") => {
                Some(WindowsHotspotObservedState {
                    active: false,
                    ownership: WindowsHotspotOwnership::None,
                    generation: None,
                    observed_at_ms: now_ms,
                })
            }
            (_, _, "On") if receipt.reason == "ownership.generation_mismatch" => {
                Some(WindowsHotspotObservedState {
                    active: true,
                    ownership: WindowsHotspotOwnership::External,
                    generation: None,
                    observed_at_ms: now_ms,
                })
            }
            (_, _, "On")
                if matches!(
                    receipt.reason.as_str(),
                    "state.restart_detected"
                        | "ownership.prior_boot_generation"
                        | "ownership.external_hotspot_on"
                ) =>
            {
                Some(WindowsHotspotObservedState {
                    active: true,
                    ownership: WindowsHotspotOwnership::External,
                    generation: None,
                    observed_at_ms: now_ms,
                })
            }
            (_, _, "Off")
                if matches!(
                    receipt.reason.as_str(),
                    "state.restart_detected" | "ownership.prior_boot_generation"
                ) =>
            {
                Some(WindowsHotspotObservedState {
                    active: false,
                    ownership: WindowsHotspotOwnership::None,
                    generation: None,
                    observed_at_ms: now_ms,
                })
            }
            (_, _, "On")
                if prior
                    .as_ref()
                    .is_some_and(|value| value.ownership == WindowsHotspotOwnership::Fleet) =>
            {
                prior
            }
            (_, _, "On") => Some(WindowsHotspotObservedState {
                active: true,
                ownership: WindowsHotspotOwnership::External,
                generation: None,
                observed_at_ms: now_ms,
            }),
            (_, _, _) if receipt.reason == "state.restart_detected" => None,
            _ => prior,
        };
        let operation = self
            .windows_hotspot_operations
            .get_mut(&receipt.operation_id)
            .expect("operation was just read");
        operation.lifecycle = if receipt.outcome == WindowsHotspotResult::Verified {
            CommandLifecycle::Applied
        } else {
            CommandLifecycle::Failed
        };
        operation.failure_code =
            (receipt.outcome != WindowsHotspotResult::Verified).then(|| receipt.reason.clone());
        operation.receipt = Some(receipt);
        operation.updated_at_ms = now_ms;
        Ok(operation.clone())
    }

    pub fn windows_hotspot_operation(
        &self,
        operation_id: &str,
    ) -> Result<WindowsHotspotOperation, HubError> {
        self.windows_hotspot_operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| {
                HubError::new(
                    "windows_hotspot_operation_not_found",
                    "Windows hotspot operation not found",
                )
            })
    }

    pub fn fail_windows_hotspot_operation(
        &mut self,
        operation_id: &str,
        failure_code: String,
        now_ms: i64,
    ) -> Result<WindowsHotspotOperation, HubError> {
        let operation = self
            .windows_hotspot_operations
            .get_mut(operation_id)
            .ok_or_else(|| {
                HubError::new("windows_hotspot_operation_not_found", "operation not found")
            })?;
        if !matches!(
            operation.lifecycle,
            CommandLifecycle::Accepted | CommandLifecycle::Dispatched | CommandLifecycle::Running
        ) {
            return Err(HubError::new(
                "windows_hotspot_failure_state_invalid",
                "operation is not in a fail-able execution state",
            ));
        }
        operation.lifecycle = CommandLifecycle::Failed;
        operation.failure_code = Some(failure_code);
        operation.updated_at_ms = now_ms;
        Ok(operation.clone())
    }

    #[must_use]
    pub fn windows_hotspot_operations(&self) -> Vec<WindowsHotspotOperation> {
        self.windows_hotspot_operations.values().cloned().collect()
    }

    pub fn recover_windows_hotspot(&mut self, now_ms: i64) -> Vec<WindowsHotspotOperation> {
        let ids = self
            .windows_hotspot_operations
            .iter()
            .filter(|(_, operation)| {
                matches!(
                    operation.lifecycle,
                    CommandLifecycle::Proposed
                        | CommandLifecycle::Accepted
                        | CommandLifecycle::Dispatched
                        | CommandLifecycle::Running
                ) && operation.preview.expires_at_ms < now_ms
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            let _ = self.expire_windows_hotspot_operation(&id, now_ms);
        }
        self.windows_hotspot_operations()
    }

    fn expire_windows_hotspot_operation(
        &mut self,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<(), HubError> {
        let operation = self
            .windows_hotspot_operations
            .get_mut(operation_id)
            .ok_or_else(|| {
                HubError::new("windows_hotspot_operation_not_found", "operation not found")
            })?;
        operation.lifecycle = CommandLifecycle::Expired;
        operation.failure_code = Some("lease_expired".to_owned());
        operation.updated_at_ms = now_ms;
        Ok(())
    }

    fn has_active_hotspot_lease(&self, now_ms: i64, except: Option<&str>) -> bool {
        self.windows_hotspot_operations.values().any(|operation| {
            except != Some(operation.operation_id.as_str())
                && matches!(
                    operation.lifecycle,
                    CommandLifecycle::Accepted
                        | CommandLifecycle::Dispatched
                        | CommandLifecycle::Running
                )
                && operation
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.expires_at_ms >= now_ms)
        })
    }
}

fn unix_ms_to_rfc3339(value: i64) -> Result<String, HubError> {
    if value < 0 {
        return Err(HubError::new(
            "windows_hotspot_deadline_invalid",
            "provider deadline precedes the Unix epoch",
        ));
    }
    let seconds = value / 1_000;
    let milliseconds = value % 1_000;
    let days = seconds / 86_400;
    let seconds_in_day = seconds % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = seconds_in_day / 3_600;
    let minute = (seconds_in_day % 3_600) / 60;
    let second = seconds_in_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}0000Z"
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WindowsHotspotObservedState {
    pub active: bool,
    pub ownership: WindowsHotspotOwnership,
    pub generation: Option<String>,
    pub observed_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HubPolicy;
    use fleet_contracts::{
        WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA, WINDOWS_HOTSPOT_PROVIDER_RECEIPT_SCHEMA,
    };

    fn plan(action: WindowsHotspotAction, suffix: &str, now_ms: i64) -> WindowsHotspotPreviewPlan {
        WindowsHotspotPreviewPlan {
            operation_id: format!("operation.{suffix}"),
            preview_id: format!("preview.{suffix}"),
            lease_id: format!("lease.{suffix}"),
            generation: format!("lease-generation.{suffix}"),
            request: WindowsHotspotPreviewRequest {
                schema: WINDOWS_HOTSPOT_PREVIEW_REQUEST_SCHEMA.to_owned(),
                action_id: WINDOWS_HOTSPOT_ACTION_ID.to_owned(),
                action,
            },
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
            provider_ready: true,
        }
    }

    fn receipt(
        invocation: &WindowsHotspotProviderRequest,
        outcome: WindowsHotspotResult,
        reason: &str,
        state: &str,
        generation: Option<&str>,
    ) -> WindowsHotspotProviderReceipt {
        WindowsHotspotProviderReceipt {
            schema: WINDOWS_HOTSPOT_PROVIDER_RECEIPT_SCHEMA.to_owned(),
            request_id: invocation.request_id.clone(),
            operation_id: invocation.operation_id.clone(),
            action: invocation.action,
            outcome,
            reason: reason.to_owned(),
            observed_at_utc: "2026-07-27T12:00:00.0000000Z".to_owned(),
            capability_available: true,
            capability: "Enabled".to_owned(),
            operational_state: state.to_owned(),
            client_count: 0,
            max_client_count: 8,
            band: "FiveGigahertz".to_owned(),
            source_connectivity: "Internet".to_owned(),
            ownership_generation: generation.map(str::to_owned),
        }
    }

    fn dispatch(
        hub: &mut FleetHub,
        action: WindowsHotspotAction,
        suffix: &str,
        now_ms: i64,
    ) -> WindowsHotspotProviderRequest {
        let operation = hub
            .preview_windows_hotspot(plan(action, suffix, now_ms))
            .expect("preview");
        hub.confirm_windows_hotspot(
            &operation.operation_id,
            &operation.preview.preview_id,
            now_ms + 1,
        )
        .expect("confirm");
        hub.prepare_windows_hotspot_invocation(
            &operation.operation_id,
            format!("request.{suffix}"),
            now_ms + 2,
        )
        .expect("prepare")
        .invocation
        .expect("invocation")
    }

    fn start_owned(hub: &mut FleetHub, suffix: &str, now_ms: i64) {
        let invocation = dispatch(hub, WindowsHotspotAction::Start, suffix, now_ms);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &invocation,
                WindowsHotspotResult::Verified,
                "start.readback_verified",
                "On",
                Some("hostess-generation.1"),
            ),
            now_ms + 3,
        )
        .expect("start receipt");
    }

    #[test]
    fn singleton_lease_rejects_concurrent_confirmation() {
        let mut hub = FleetHub::new(HubPolicy::default());
        let one = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Status, "one", 1_000))
            .expect("one");
        let two = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Status, "two", 1_000))
            .expect("two");
        hub.confirm_windows_hotspot(&one.operation_id, &one.preview.preview_id, 1_001)
            .expect("first lease");
        assert_eq!(
            hub.confirm_windows_hotspot(&two.operation_id, &two.preview.preview_id, 1_002)
                .expect_err("concurrent lease")
                .code,
            "windows_hotspot_resource_leased"
        );
    }

    #[test]
    fn provider_generation_is_retained_and_exact_stop_generation_is_required() {
        let mut hub = FleetHub::new(HubPolicy::default());
        start_owned(&mut hub, "start", 1_000);
        let stop = dispatch(&mut hub, WindowsHotspotAction::Stop, "stop", 2_000);
        assert_eq!(
            stop.ownership_generation.as_deref(),
            Some("hostess-generation.1")
        );
        hub.apply_windows_hotspot_receipt(
            receipt(
                &stop,
                WindowsHotspotResult::Verified,
                "stop.readback_verified",
                "Off",
                None,
            ),
            2_003,
        )
        .expect("stop receipt");
        let next = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Stop, "next-stop", 3_000))
            .expect("ineligible preview");
        assert_eq!(next.lifecycle, CommandLifecycle::Rejected);
    }

    #[test]
    fn external_hotspot_is_observed_but_never_taken_over() {
        let mut hub = FleetHub::new(HubPolicy::default());
        let status = dispatch(&mut hub, WindowsHotspotAction::Status, "status", 1_000);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &status,
                WindowsHotspotResult::Verified,
                "status.read",
                "On",
                None,
            ),
            1_003,
        )
        .expect("status");
        let start = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Start, "start", 2_000))
            .expect("preview");
        assert_eq!(start.lifecycle, CommandLifecycle::Rejected);
        assert_eq!(
            start.preview.preflight.ownership,
            WindowsHotspotOwnership::External
        );
    }

    #[test]
    fn restart_receipt_invalidates_old_generation_but_allows_explicit_start_recovery() {
        let mut hub = FleetHub::new(HubPolicy::default());
        start_owned(&mut hub, "start", 1_000);
        let status = dispatch(&mut hub, WindowsHotspotAction::Status, "status", 2_000);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &status,
                WindowsHotspotResult::Failed,
                "state.restart_detected",
                "Unknown",
                None,
            ),
            2_003,
        )
        .expect("restart receipt");
        let stop = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Stop, "stop", 3_000))
            .expect("stop preview");
        assert_eq!(stop.lifecycle, CommandLifecycle::Rejected);
        let start = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Start, "recover", 3_000))
            .expect("start preview");
        assert_eq!(start.lifecycle, CommandLifecycle::Proposed);
    }

    #[test]
    fn durable_restore_expires_abandoned_host_lease() {
        let mut hub = FleetHub::new(HubPolicy::default());
        let operation = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Status, "restore", 1_000))
            .expect("preview");
        hub.confirm_windows_hotspot(
            &operation.operation_id,
            &operation.preview.preview_id,
            1_001,
        )
        .expect("confirm");
        let snapshot = hub.snapshot();
        let mut restored = FleetHub::restore(HubPolicy::default(), snapshot).expect("restore");
        restored.recover_windows_hotspot(70_000);
        assert_eq!(
            restored
                .windows_hotspot_operation(&operation.operation_id)
                .expect("operation")
                .lifecycle,
            CommandLifecycle::Expired
        );
    }

    #[test]
    fn ensure_cannot_cross_the_generation_frozen_in_its_preview() {
        let mut hub = FleetHub::new(HubPolicy::default());
        start_owned(&mut hub, "start-one", 1_000);
        let ensure = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Ensure, "ensure", 2_000))
            .expect("ensure preview");
        let stop = dispatch(&mut hub, WindowsHotspotAction::Stop, "stop", 3_000);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &stop,
                WindowsHotspotResult::Verified,
                "stop.readback_verified",
                "Off",
                None,
            ),
            3_003,
        )
        .expect("stop");
        let start_two = dispatch(&mut hub, WindowsHotspotAction::Start, "start-two", 4_000);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &start_two,
                WindowsHotspotResult::Verified,
                "start.readback_verified",
                "On",
                Some("hostess-generation.2"),
            ),
            4_003,
        )
        .expect("second start");
        hub.confirm_windows_hotspot(&ensure.operation_id, &ensure.preview.preview_id, 5_000)
            .expect("confirm old preview");
        assert_eq!(
            hub.prepare_windows_hotspot_invocation(
                &ensure.operation_id,
                "request.ensure".to_owned(),
                5_001,
            )
            .expect_err("generation drift")
            .code,
            "windows_hotspot_generation_changed"
        );
    }

    #[test]
    fn nonverified_receipt_cannot_manufacture_fleet_ownership() {
        let mut hub = FleetHub::new(HubPolicy::default());
        let start = dispatch(&mut hub, WindowsHotspotAction::Start, "failed-start", 1_000);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &start,
                WindowsHotspotResult::Failed,
                "start.state_write_failed",
                "On",
                Some("untrusted-generation"),
            ),
            1_003,
        )
        .expect("failed receipt");
        let stop = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Stop, "stop", 2_000))
            .expect("stop preview");
        assert_eq!(stop.lifecycle, CommandLifecycle::Rejected);
        assert_eq!(
            stop.preview.preflight.ownership,
            WindowsHotspotOwnership::External
        );
    }

    #[test]
    fn terminal_receipt_replay_cannot_resurrect_an_old_generation() {
        let mut hub = FleetHub::new(HubPolicy::default());
        let start = dispatch(&mut hub, WindowsHotspotAction::Start, "start", 1_000);
        let start_receipt = receipt(
            &start,
            WindowsHotspotResult::Verified,
            "start.readback_verified",
            "On",
            Some("hostess-generation.1"),
        );
        hub.apply_windows_hotspot_receipt(start_receipt.clone(), 1_003)
            .expect("start");
        let stop = dispatch(&mut hub, WindowsHotspotAction::Stop, "stop", 2_000);
        hub.apply_windows_hotspot_receipt(
            receipt(
                &stop,
                WindowsHotspotResult::Verified,
                "stop.readback_verified",
                "Off",
                None,
            ),
            2_003,
        )
        .expect("stop");
        assert_eq!(
            hub.apply_windows_hotspot_receipt(start_receipt, 3_000)
                .expect_err("receipt replay")
                .code,
            "windows_hotspot_receipt_replay"
        );
        let next_stop = hub
            .preview_windows_hotspot(plan(WindowsHotspotAction::Stop, "next-stop", 4_000))
            .expect("stop preview");
        assert_eq!(next_stop.lifecycle, CommandLifecycle::Rejected);
    }

    #[test]
    fn generation_mismatch_invalidates_prior_or_absent_ownership() {
        let mut owned = FleetHub::new(HubPolicy::default());
        start_owned(&mut owned, "owned-start", 1_000);
        let ensure = dispatch(
            &mut owned,
            WindowsHotspotAction::Ensure,
            "owned-ensure",
            2_000,
        );
        owned
            .apply_windows_hotspot_receipt(
                receipt(
                    &ensure,
                    WindowsHotspotResult::Rejected,
                    "ownership.generation_mismatch",
                    "On",
                    None,
                ),
                2_003,
            )
            .expect("mismatch receipt");
        assert_eq!(
            owned
                .windows_hotspot_observation
                .as_ref()
                .expect("observation")
                .ownership,
            WindowsHotspotOwnership::External
        );

        let mut absent = FleetHub::new(HubPolicy::default());
        let ensure = dispatch(
            &mut absent,
            WindowsHotspotAction::Ensure,
            "absent-ensure",
            3_000,
        );
        absent
            .apply_windows_hotspot_receipt(
                receipt(
                    &ensure,
                    WindowsHotspotResult::Rejected,
                    "ownership.generation_mismatch",
                    "On",
                    None,
                ),
                3_003,
            )
            .expect("mismatch receipt");
        assert_eq!(
            absent
                .windows_hotspot_observation
                .as_ref()
                .expect("observation")
                .ownership,
            WindowsHotspotOwnership::External
        );
    }

    #[test]
    fn verified_status_preserves_only_omitted_or_matching_generation() {
        let mut hub = FleetHub::new(HubPolicy::default());
        start_owned(&mut hub, "start", 1_000);
        for (suffix, generation, expected) in [
            ("status-omitted", None, WindowsHotspotOwnership::Fleet),
            (
                "status-matching",
                Some("hostess-generation.1"),
                WindowsHotspotOwnership::Fleet,
            ),
            (
                "status-changed",
                Some("hostess-generation.2"),
                WindowsHotspotOwnership::External,
            ),
        ] {
            let status = dispatch(&mut hub, WindowsHotspotAction::Status, suffix, 2_000);
            hub.apply_windows_hotspot_receipt(
                receipt(
                    &status,
                    WindowsHotspotResult::Verified,
                    "status.read",
                    "On",
                    generation,
                ),
                2_003,
            )
            .expect("status receipt");
            assert_eq!(
                hub.windows_hotspot_observation
                    .as_ref()
                    .expect("observation")
                    .ownership,
                expected
            );
        }
    }
}
