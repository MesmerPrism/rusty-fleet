// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic, in-memory Fleet Hub state engine.
//!
//! The crate deliberately contains no persistence, socket, device, or adapter
//! implementation. Callers supply accepted observations and the current time.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

use fleet_contracts::{
    Comparison, ConditionFamily, ConditionState, DeviceDetailProjection, DeviceInspectorProjection,
    DeviceObservation, DeviceRowProjection, FleetQuery, FleetQueryResult, FleetSummaryProjection,
    KioskState, ProjectionFreshness, QueryExpression, QueryField, QueryValue, SortDirection,
    StatusCondition, ValidateContract,
};
use serde::{Deserialize, Serialize};

const ROW_SCHEMA: &str = "rusty.fleet.device_row.v1";
const INSPECTOR_SCHEMA: &str = "rusty.fleet.device_inspector.v1";
const DETAIL_SCHEMA: &str = "rusty.fleet.device_detail.v1";
const SUMMARY_SCHEMA: &str = "rusty.fleet.summary.v1";
const RESULT_SCHEMA: &str = "rusty.fleet.query_result.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HubPolicy {
    pub stale_after_ms: i64,
    pub offline_after_ms: i64,
    pub history_limit_per_device: usize,
    pub event_limit: usize,
}

impl Default for HubPolicy {
    fn default() -> Self {
        Self {
            stale_after_ms: 60_000,
            offline_after_ms: 300_000,
            history_limit_per_device: 128,
            event_limit: 10_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    ContractInvalid,
    DuplicateRevision,
    StaleRevision,
    IdentityRevisionRollback,
    IdentityRevisionChangedWithoutRestart,
    IdentityConflict,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ObservationDecision {
    Accepted {
        result_revision: u64,
        device_id: String,
        source_revision: u64,
    },
    Rejected {
        result_revision: u64,
        device_id: Option<String>,
        source_revision: Option<u64>,
        reason: RejectionReason,
        details: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchEvent {
    pub schema: String,
    pub event_sequence: u64,
    pub observed_at_ms: i64,
    pub decision: ObservationDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubError {
    pub code: String,
    pub message: String,
}

impl HubError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl Display for HubError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HubError {}

/// The local, in-process API shared by `fleetctl` and future UI projections.
pub trait FleetApi {
    fn list(&self, query: &FleetQuery, now_ms: i64) -> Result<FleetQueryResult, HubError>;
    fn inspect(&self, device_id: &str, now_ms: i64) -> Result<DeviceInspectorProjection, HubError>;
    fn detail(&self, device_id: &str, now_ms: i64) -> Result<DeviceDetailProjection, HubError>;
    fn summary(&self, now_ms: i64) -> FleetSummaryProjection;
    fn watch(&self, after_sequence: u64, limit: usize) -> Vec<WatchEvent>;
}

#[derive(Clone, Debug)]
struct DeviceRecord {
    observation: DeviceObservation,
    accepted_at_ms: i64,
    accepted_revision: u64,
    condition_history: Vec<StatusCondition>,
}

#[derive(Clone, Debug)]
pub struct FleetHub {
    policy: HubPolicy,
    devices: BTreeMap<String, DeviceRecord>,
    result_revision: u64,
    event_sequence: u64,
    events: Vec<WatchEvent>,
}

impl FleetHub {
    #[must_use]
    pub fn new(policy: HubPolicy) -> Self {
        assert!(
            policy.stale_after_ms > 0
                && policy.offline_after_ms > policy.stale_after_ms
                && policy.history_limit_per_device > 0
                && policy.event_limit > 0,
            "Hub policy must contain positive, ordered, finite limits"
        );
        Self {
            policy,
            devices: BTreeMap::new(),
            result_revision: 1,
            event_sequence: 0,
            events: Vec::new(),
        }
    }

    #[must_use]
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    #[must_use]
    pub fn result_revision(&self) -> u64 {
        self.result_revision
    }

    pub fn accept_observation(
        &mut self,
        observation: DeviceObservation,
        now_ms: i64,
    ) -> ObservationDecision {
        let validation = observation.validate();
        if let Err(failures) = validation {
            let decision = ObservationDecision::Rejected {
                result_revision: self.result_revision,
                device_id: nonempty(&observation.identity.device_id),
                source_revision: (observation.source_revision > 0)
                    .then_some(observation.source_revision),
                reason: RejectionReason::ContractInvalid,
                details: failures
                    .into_iter()
                    .map(|failure| format!("{}:{}:{}", failure.code, failure.path, failure.message))
                    .collect(),
            };
            self.record_event(now_ms, decision.clone());
            return decision;
        }

        let device_id = observation.identity.device_id.clone();
        if let Some(existing) = self.devices.get(&device_id) {
            let rejection = if observation.identity.identity_revision
                == existing.observation.identity.identity_revision
                && (observation.identity.display_name != existing.observation.identity.display_name
                    || observation.identity.model != existing.observation.identity.model
                    || observation.identity.hardware_class
                        != existing.observation.identity.hardware_class)
            {
                Some(RejectionReason::IdentityConflict)
            } else if observation.identity.identity_revision
                < existing.observation.identity.identity_revision
            {
                Some(RejectionReason::IdentityRevisionRollback)
            } else if observation.identity.identity_revision
                > existing.observation.identity.identity_revision
                && observation.source_revision != 1
            {
                Some(RejectionReason::IdentityRevisionChangedWithoutRestart)
            } else if observation.identity.identity_revision
                == existing.observation.identity.identity_revision
                && observation.source_revision == existing.observation.source_revision
            {
                Some(RejectionReason::DuplicateRevision)
            } else if observation.identity.identity_revision
                == existing.observation.identity.identity_revision
                && observation.source_revision < existing.observation.source_revision
            {
                Some(RejectionReason::StaleRevision)
            } else {
                None
            };

            if let Some(reason) = rejection {
                let decision = ObservationDecision::Rejected {
                    result_revision: self.result_revision,
                    device_id: Some(device_id),
                    source_revision: Some(observation.source_revision),
                    reason,
                    details: Vec::new(),
                };
                self.record_event(now_ms, decision.clone());
                return decision;
            }
        }

        self.result_revision = self.result_revision.saturating_add(1);
        let mut history = self
            .devices
            .get(&device_id)
            .map_or_else(Vec::new, |record| record.condition_history.clone());
        history.extend(observation.conditions.iter().cloned());
        if history.len() > self.policy.history_limit_per_device {
            let keep_from = history.len() - self.policy.history_limit_per_device;
            history.drain(..keep_from);
        }
        let source_revision = observation.source_revision;
        self.devices.insert(
            device_id.clone(),
            DeviceRecord {
                observation,
                accepted_at_ms: now_ms,
                accepted_revision: self.result_revision,
                condition_history: history,
            },
        );
        let decision = ObservationDecision::Accepted {
            result_revision: self.result_revision,
            device_id,
            source_revision,
        };
        self.record_event(now_ms, decision.clone());
        decision
    }

    fn record_event(&mut self, observed_at_ms: i64, decision: ObservationDecision) {
        self.event_sequence = self.event_sequence.saturating_add(1);
        self.events.push(WatchEvent {
            schema: "rusty.fleet.watch_event.v1".to_owned(),
            event_sequence: self.event_sequence,
            observed_at_ms,
            decision,
        });
        if self.events.len() > self.policy.event_limit {
            let remove = self.events.len() - self.policy.event_limit;
            self.events.drain(..remove);
        }
    }

    fn row(&self, record: &DeviceRecord, now_ms: i64) -> DeviceRowProjection {
        let age_ms = now_ms.saturating_sub(record.accepted_at_ms).max(0);
        let freshness = if age_ms > self.policy.offline_after_ms {
            ProjectionFreshness::Offline
        } else if age_ms > self.policy.stale_after_ms {
            ProjectionFreshness::Stale
        } else {
            ProjectionFreshness::Fresh
        };
        let conditions = newest_conditions(&record.observation.conditions, now_ms);
        DeviceRowProjection {
            schema: ROW_SCHEMA.to_owned(),
            identity: record.observation.identity.clone(),
            accepted_revision: record.accepted_revision,
            accepted_at_ms: record.accepted_at_ms,
            age_ms,
            freshness,
            battery_percent: record.observation.battery_percent,
            charging: record.observation.charging,
            foreground_app: record.observation.foreground_app.clone(),
            kiosk_state: kiosk_text(record.observation.kiosk_state).to_owned(),
            route: route_text(&record.observation, freshness),
            conditions,
            capabilities: record.observation.capabilities.clone(),
            stream_count: record.observation.streams.len(),
            active_work_count: 0,
            extensions: BTreeMap::new(),
        }
    }
}

impl FleetApi for FleetHub {
    fn list(&self, query: &FleetQuery, now_ms: i64) -> Result<FleetQueryResult, HubError> {
        query.validate().map_err(|failures| {
            HubError::new(
                "invalid_query",
                failures
                    .into_iter()
                    .map(|failure| failure.code)
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })?;
        let mut rows: Vec<_> = self
            .devices
            .values()
            .map(|record| self.row(record, now_ms))
            .filter(|row| {
                query
                    .expression
                    .as_ref()
                    .is_none_or(|expression| evaluate(expression, row))
            })
            .collect();
        rows.sort_by(|left, right| compare_rows(left, right, query));
        let total_count = rows.len();
        let offset = query.offset.min(total_count);
        let end = offset.saturating_add(query.limit).min(total_count);
        let rows = rows[offset..end].to_vec();
        Ok(FleetQueryResult {
            schema: RESULT_SCHEMA.to_owned(),
            query: query.clone(),
            result_revision: self.result_revision,
            as_of_ms: now_ms,
            total_count,
            window_offset: offset,
            window_count: rows.len(),
            rows,
        })
    }

    fn inspect(&self, device_id: &str, now_ms: i64) -> Result<DeviceInspectorProjection, HubError> {
        let record = self.devices.get(device_id).ok_or_else(|| {
            HubError::new("device_not_found", format!("unknown device {device_id}"))
        })?;
        let row = self.row(record, now_ms);
        let attention = row
            .conditions
            .values()
            .filter(|condition| {
                condition.is_stale_at(now_ms)
                    || matches!(
                        condition.state,
                        ConditionState::Degraded
                            | ConditionState::Failed
                            | ConditionState::Critical
                    )
            })
            .cloned()
            .collect();
        Ok(DeviceInspectorProjection {
            schema: INSPECTOR_SCHEMA.to_owned(),
            row,
            attention,
            streams: record.observation.streams.clone(),
            active_operations: Vec::new(),
        })
    }

    fn detail(&self, device_id: &str, now_ms: i64) -> Result<DeviceDetailProjection, HubError> {
        let record = self.devices.get(device_id).ok_or_else(|| {
            HubError::new("device_not_found", format!("unknown device {device_id}"))
        })?;
        Ok(DeviceDetailProjection {
            schema: DETAIL_SCHEMA.to_owned(),
            inspector: self.inspect(device_id, now_ms)?,
            condition_history: record.condition_history.clone(),
            operation_history: Vec::new(),
        })
    }

    fn summary(&self, now_ms: i64) -> FleetSummaryProjection {
        let rows: Vec<_> = self
            .devices
            .values()
            .map(|record| self.row(record, now_ms))
            .collect();
        FleetSummaryProjection {
            schema: SUMMARY_SCHEMA.to_owned(),
            as_of_ms: now_ms,
            total: rows.len(),
            fresh: rows
                .iter()
                .filter(|row| row.freshness == ProjectionFreshness::Fresh)
                .count(),
            stale: rows
                .iter()
                .filter(|row| row.freshness == ProjectionFreshness::Stale)
                .count(),
            offline: rows
                .iter()
                .filter(|row| row.freshness == ProjectionFreshness::Offline)
                .count(),
            attention: rows
                .iter()
                .filter(|row| {
                    row.conditions.values().any(|condition| {
                        condition.is_stale_at(now_ms)
                            || matches!(
                                condition.state,
                                ConditionState::Degraded
                                    | ConditionState::Failed
                                    | ConditionState::Critical
                            )
                    })
                })
                .count(),
            active_work: rows.iter().map(|row| row.active_work_count).sum(),
        }
    }

    fn watch(&self, after_sequence: u64, limit: usize) -> Vec<WatchEvent> {
        self.events
            .iter()
            .filter(|event| event.event_sequence > after_sequence)
            .take(limit.min(self.policy.event_limit))
            .cloned()
            .collect()
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn kiosk_text(state: KioskState) -> &'static str {
    match state {
        KioskState::Active => "active",
        KioskState::Inactive => "inactive",
        KioskState::Mismatch => "mismatch",
        KioskState::Unknown => "unknown",
    }
}

fn route_text(observation: &DeviceObservation, freshness: ProjectionFreshness) -> String {
    if freshness == ProjectionFreshness::Offline {
        return "offline".to_owned();
    }
    observation
        .conditions
        .iter()
        .find(|condition| condition.family == ConditionFamily::Freshness)
        .map_or_else(
            || "unknown".to_owned(),
            |condition| condition.reason.clone(),
        )
}

fn newest_conditions(
    conditions: &[StatusCondition],
    now_ms: i64,
) -> BTreeMap<ConditionFamily, StatusCondition> {
    let mut newest = BTreeMap::new();
    for condition in conditions {
        let mut projected = condition.clone();
        if projected.is_stale_at(now_ms)
            && !matches!(
                projected.state,
                ConditionState::Failed | ConditionState::Critical | ConditionState::Restricted
            )
        {
            projected.state = ConditionState::Stale;
        }
        let should_replace = newest
            .get(&projected.family)
            .is_none_or(|prior: &StatusCondition| {
                (projected.accepted_revision, projected.received_time_ms)
                    > (prior.accepted_revision, prior.received_time_ms)
            });
        if should_replace {
            newest.insert(projected.family, projected);
        }
    }
    newest
}

fn evaluate(expression: &QueryExpression, row: &DeviceRowProjection) -> bool {
    match expression {
        QueryExpression::Predicate {
            field,
            comparison,
            value,
            qualifier,
        } => evaluate_predicate(
            *field,
            *comparison,
            value.as_ref(),
            qualifier.as_deref(),
            row,
        ),
        QueryExpression::And { expressions } => expressions
            .iter()
            .all(|expression| evaluate(expression, row)),
        QueryExpression::Or { expressions } => expressions
            .iter()
            .any(|expression| evaluate(expression, row)),
        QueryExpression::Not { expression } => !evaluate(expression, row),
    }
}

fn evaluate_predicate(
    field: QueryField,
    comparison: Comparison,
    value: Option<&QueryValue>,
    qualifier: Option<&str>,
    row: &DeviceRowProjection,
) -> bool {
    match field {
        QueryField::DeviceId => compare_text(&row.identity.device_id, comparison, value),
        QueryField::DisplayName => compare_text(&row.identity.display_name, comparison, value),
        QueryField::Model => compare_text(&row.identity.model, comparison, value),
        QueryField::Tag => qualifier
            .and_then(|key| row.identity.tags.get(key))
            .is_some_and(|actual| compare_text(actual, comparison, value)),
        QueryField::Freshness => compare_text(freshness_text(row.freshness), comparison, value),
        QueryField::BatteryPercent => {
            compare_optional_integer(row.battery_percent.map(i64::from), comparison, value)
        }
        QueryField::ForegroundApp => row
            .foreground_app
            .as_deref()
            .is_some_and(|actual| compare_text(actual, comparison, value)),
        QueryField::KioskState => compare_text(&row.kiosk_state, comparison, value),
        QueryField::Capability => qualifier
            .and_then(|capability_id| row.capabilities.get(capability_id))
            .is_some_and(|capability| {
                if comparison == Comparison::Exists {
                    true
                } else {
                    compare_bool(capability.is_ready(), comparison, value)
                }
            }),
        QueryField::ConditionFamily => condition_values(row)
            .iter()
            .any(|value_text| compare_text(value_text, comparison, value)),
        QueryField::ConditionState => {
            let states: BTreeSet<_> = row
                .conditions
                .values()
                .map(|condition| condition_state_text(condition.state))
                .collect();
            states
                .iter()
                .any(|state| compare_text(state, comparison, value))
        }
    }
}

fn condition_values(row: &DeviceRowProjection) -> Vec<&'static str> {
    row.conditions
        .keys()
        .map(|family| match family {
            ConditionFamily::Identity => "identity",
            ConditionFamily::Freshness => "freshness",
            ConditionFamily::Power => "power",
            ConditionFamily::Application => "application",
            ConditionFamily::Control => "control",
            ConditionFamily::Privileged => "privileged",
            ConditionFamily::Media => "media",
            ConditionFamily::Work => "work",
            ConditionFamily::Alert => "alert",
        })
        .collect()
}

fn condition_state_text(state: ConditionState) -> &'static str {
    match state {
        ConditionState::Current => "current",
        ConditionState::InProgress => "in_progress",
        ConditionState::Busy => "busy",
        ConditionState::Stale => "stale",
        ConditionState::Unknown => "unknown",
        ConditionState::Unsupported => "unsupported",
        ConditionState::Disabled => "disabled",
        ConditionState::Unauthorized => "unauthorized",
        ConditionState::Disconnected => "disconnected",
        ConditionState::Unavailable => "unavailable",
        ConditionState::Degraded => "degraded",
        ConditionState::Failed => "failed",
        ConditionState::Critical => "critical",
        ConditionState::Restricted => "restricted",
    }
}

fn freshness_text(freshness: ProjectionFreshness) -> &'static str {
    match freshness {
        ProjectionFreshness::Fresh => "fresh",
        ProjectionFreshness::Stale => "stale",
        ProjectionFreshness::Offline => "offline",
        ProjectionFreshness::Unknown => "unknown",
    }
}

fn compare_text(actual: &str, comparison: Comparison, value: Option<&QueryValue>) -> bool {
    if comparison == Comparison::Exists {
        return !actual.is_empty();
    }
    let actual = actual.to_lowercase();
    match value {
        Some(QueryValue::Text(expected)) => compare_ordering(
            actual.cmp(&expected.to_lowercase()),
            comparison,
            actual.contains(&expected.to_lowercase()),
        ),
        Some(QueryValue::TextList(expected)) => {
            let equal = expected.iter().any(|item| actual == item.to_lowercase());
            matches!(comparison, Comparison::Equals) && equal
                || matches!(comparison, Comparison::NotEquals) && !equal
        }
        _ => false,
    }
}

fn compare_optional_integer(
    actual: Option<i64>,
    comparison: Comparison,
    value: Option<&QueryValue>,
) -> bool {
    if comparison == Comparison::Exists {
        return actual.is_some();
    }
    match (actual, value) {
        (Some(actual), Some(QueryValue::Integer(expected))) => {
            compare_ordering(actual.cmp(expected), comparison, false)
        }
        _ => false,
    }
}

fn compare_bool(actual: bool, comparison: Comparison, value: Option<&QueryValue>) -> bool {
    match value {
        Some(QueryValue::Boolean(expected)) => {
            comparison == Comparison::Equals && actual == *expected
                || comparison == Comparison::NotEquals && actual != *expected
        }
        _ => false,
    }
}

fn compare_ordering(ordering: Ordering, comparison: Comparison, contains: bool) -> bool {
    match comparison {
        Comparison::Equals => ordering == Ordering::Equal,
        Comparison::NotEquals => ordering != Ordering::Equal,
        Comparison::Contains => contains,
        Comparison::LessThan => ordering == Ordering::Less,
        Comparison::LessThanOrEqual => ordering != Ordering::Greater,
        Comparison::GreaterThan => ordering == Ordering::Greater,
        Comparison::GreaterThanOrEqual => ordering != Ordering::Less,
        Comparison::Exists => true,
    }
}

fn compare_rows(
    left: &DeviceRowProjection,
    right: &DeviceRowProjection,
    query: &FleetQuery,
) -> Ordering {
    for key in &query.sort {
        let ordering = match key.field {
            QueryField::DeviceId => left.identity.device_id.cmp(&right.identity.device_id),
            QueryField::DisplayName => left.identity.display_name.cmp(&right.identity.display_name),
            QueryField::Model => left.identity.model.cmp(&right.identity.model),
            QueryField::Freshness => {
                freshness_rank(left.freshness).cmp(&freshness_rank(right.freshness))
            }
            QueryField::BatteryPercent => left.battery_percent.cmp(&right.battery_percent),
            QueryField::ForegroundApp => left.foreground_app.cmp(&right.foreground_app),
            QueryField::KioskState => left.kiosk_state.cmp(&right.kiosk_state),
            QueryField::Tag => key
                .qualifier
                .as_ref()
                .map(|qualifier| {
                    left.identity
                        .tags
                        .get(qualifier)
                        .cmp(&right.identity.tags.get(qualifier))
                })
                .unwrap_or(Ordering::Equal),
            QueryField::Capability | QueryField::ConditionFamily | QueryField::ConditionState => {
                Ordering::Equal
            }
        };
        let ordering = match key.direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.identity
        .display_name
        .cmp(&right.identity.display_name)
        .then_with(|| left.identity.device_id.cmp(&right.identity.device_id))
}

fn freshness_rank(freshness: ProjectionFreshness) -> u8 {
    match freshness {
        ProjectionFreshness::Fresh => 0,
        ProjectionFreshness::Stale => 1,
        ProjectionFreshness::Offline => 2,
        ProjectionFreshness::Unknown => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::{FleetApi, FleetHub, HubPolicy, ObservationDecision, RejectionReason};
    use fleet_contracts::{
        Comparison, FleetQuery, QueryExpression, QueryField, QueryValue, ValidateContract,
    };
    use fleet_simulator::{BASE_TIME_MS, ScenarioBuilder};

    fn all_query(limit: usize) -> FleetQuery {
        FleetQuery {
            schema: "rusty.fleet.query.v1".to_owned(),
            query_id: "all".to_owned(),
            expression: None,
            sort: Vec::new(),
            offset: 0,
            limit,
        }
    }

    #[test]
    fn accepts_independent_devices_and_rejects_replay() {
        let scenario = ScenarioBuilder::new(4).build();
        let mut hub = FleetHub::new(HubPolicy::default());
        for observation in scenario.initial {
            assert!(matches!(
                hub.accept_observation(observation, BASE_TIME_MS),
                ObservationDecision::Accepted { .. }
            ));
        }
        assert_eq!(hub.device_count(), 4);

        let replay = ScenarioBuilder::new(1).build().initial.remove(0);
        let decision = hub.accept_observation(replay, BASE_TIME_MS + 1);
        assert!(matches!(
            decision,
            ObservationDecision::Rejected {
                reason: RejectionReason::DuplicateRevision,
                ..
            }
        ));
    }

    #[test]
    fn scenario_mutations_preserve_identity_and_revision_rules() {
        let scenario = ScenarioBuilder::new(4).build();
        let mut hub = FleetHub::new(HubPolicy::default());
        for observation in scenario.initial {
            hub.accept_observation(observation, BASE_TIME_MS);
        }
        let mut reasons = Vec::new();
        for mutation in scenario.mutations {
            if let ObservationDecision::Rejected { reason, .. } =
                hub.accept_observation(mutation.observation, mutation.at_ms)
            {
                reasons.push(reason);
            }
        }
        assert!(reasons.contains(&RejectionReason::StaleRevision));
        assert!(reasons.contains(&RejectionReason::IdentityConflict));
        assert!(reasons.contains(&RejectionReason::ContractInvalid));
        assert_eq!(hub.device_count(), 4);
    }

    #[test]
    fn stale_and_offline_are_time_projections_not_deleted_devices() {
        let scenario = ScenarioBuilder::new(4).build();
        let mut hub = FleetHub::new(HubPolicy::default());
        for observation in scenario.initial {
            hub.accept_observation(observation, BASE_TIME_MS);
        }
        assert_eq!(hub.summary(BASE_TIME_MS).fresh, 4);
        assert_eq!(hub.summary(BASE_TIME_MS + 61_000).stale, 4);
        assert_eq!(hub.summary(BASE_TIME_MS + 301_000).offline, 4);
        assert_eq!(hub.device_count(), 4);
    }

    #[test]
    fn canonical_projections_validate_at_the_api_boundary() {
        let scenario = ScenarioBuilder::new(4).build();
        let mut hub = FleetHub::new(HubPolicy::default());
        for observation in scenario.initial {
            hub.accept_observation(observation, BASE_TIME_MS);
        }
        assert!(hub.summary(BASE_TIME_MS).validate().is_ok());
        assert!(
            hub.list(&all_query(4), BASE_TIME_MS)
                .expect("query")
                .validate()
                .is_ok()
        );
        assert!(
            hub.inspect("sim-00001", BASE_TIME_MS)
                .expect("inspector")
                .validate()
                .is_ok()
        );
        assert!(
            hub.detail("sim-00001", BASE_TIME_MS)
                .expect("detail")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn query_and_window_are_deterministic() {
        let scenario = ScenarioBuilder::new(50).build();
        let mut hub = FleetHub::new(HubPolicy::default());
        for observation in scenario.initial {
            hub.accept_observation(observation, BASE_TIME_MS);
        }
        let query = FleetQuery {
            expression: Some(QueryExpression::Predicate {
                field: QueryField::Tag,
                comparison: Comparison::Equals,
                value: Some(QueryValue::Text("lab-a".to_owned())),
                qualifier: Some("cohort".to_owned()),
            }),
            limit: 7,
            ..all_query(7)
        };
        let first = hub.list(&query, BASE_TIME_MS).expect("valid query");
        let second = hub.list(&query, BASE_TIME_MS).expect("valid query");
        assert_eq!(first, second);
        assert_eq!(first.window_count, 7);
        assert_eq!(first.total_count, 25);
    }
}
