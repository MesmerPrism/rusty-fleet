// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ConditionState, ContractViolation, Sensitivity, ValidateContract, finish, require_nonempty,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamPlane {
    Control,
    Observation,
    Media,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamSemantic {
    pub family: String,
    pub class: String,
    pub schema_profile: String,
    pub plane: StreamPlane,
    pub sensitivity: Sensitivity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeDescriptor {
    pub kind: String,
    pub adapter_version: String,
    pub native_protocol_version: String,
    pub digest_sha256: String,
    pub captured_at_ms: i64,
    pub document: Option<Value>,
    pub reference: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMethod {
    SavedRule,
    ManualPin,
    OwnerManifestExactMatch,
    SingleCandidate,
    PolicyTiebreak,
    UnresolvedAmbiguous,
    NoCandidate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSelection {
    pub query_language: String,
    pub query: String,
    pub expected_cardinality: usize,
    pub candidate_count: usize,
    #[serde(default)]
    pub candidate_descriptor_digests: Vec<String>,
    pub chosen_native_instance: Option<String>,
    pub method: SelectionMethod,
    pub valid_until_ms: i64,
    pub override_actor: Option<String>,
    pub override_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpochContinuity {
    Continuous,
    SourceContinuityProven,
    Discontinuous,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentEpoch {
    pub id: String,
    pub predecessor: Option<String>,
    pub reason: String,
    pub continuity: EpochContinuity,
    pub native_instance: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentEpochs {
    pub source: ComponentEpoch,
    pub route: Option<ComponentEpoch>,
    pub processing: Option<ComponentEpoch>,
    pub sink: Option<ComponentEpoch>,
    pub path_generation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingDomain {
    pub domain_id: String,
    pub domain_kind: String,
    pub units: String,
    pub time_base_numerator: i64,
    pub time_base_denominator: i64,
    pub raw_timestamp_preserved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimingTransform {
    pub transform_id: String,
    pub from_domain: String,
    pub to_domain: String,
    pub method: String,
    pub offset_ms: f64,
    pub uncertainty_ms: f64,
    pub valid_from_ms: i64,
    pub valid_to_ms: Option<i64>,
    #[serde(default)]
    pub postprocessing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimingCorrelation {
    #[serde(default)]
    pub domains: Vec<TimingDomain>,
    #[serde(default)]
    pub transforms: Vec<TimingTransform>,
    pub clock_reset_count: u64,
    pub fixed_latency_ms: Option<f64>,
    pub fixed_latency_uncertainty_ms: Option<f64>,
    pub calibration_reference: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CadenceMode {
    Regular,
    Irregular,
    EventDriven,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CadencePolicy {
    pub mode: CadenceMode,
    pub nominal_rate_hz: Option<f64>,
    pub accepted_rate_min_hz: Option<f64>,
    pub accepted_rate_max_hz: Option<f64>,
    pub measurement_window_ms: u64,
    pub gap_tolerance_ms: Option<u64>,
    pub no_data_deadline_ms: Option<u64>,
    pub heartbeat: Option<String>,
    pub sequence_semantics: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStage {
    SourceReceipt,
    Admission,
    Route,
    Process,
    Bytes,
    CompletePayload,
    DecodeOrSchema,
    Sink,
    Recording,
    Cleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressApplicability {
    Required,
    Optional,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentProgressPolicy {
    Sequence,
    Timestamp,
    ChangingIdentity,
    ExpectedStatic,
    SemanticValue,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgressStageEvidence {
    pub applicability: ProgressApplicability,
    pub deadline_ms: Option<u64>,
    pub state: Option<ConditionState>,
    pub observed_revision: Option<u64>,
    pub last_progress_ms: Option<i64>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgressProfile {
    pub profile_id: String,
    pub content_progress: ContentProgressPolicy,
    #[serde(default)]
    pub stages: BTreeMap<ProgressStage, ProgressStageEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueLimits {
    pub items: usize,
    pub bytes: usize,
    pub duration_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowPolicy {
    Reject,
    BlockProducer,
    DropOldest,
    DropNewest,
    FailConsumer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeQueuePolicy {
    pub edge_id: String,
    pub producer: String,
    pub consumer: String,
    pub limits: QueueLimits,
    pub overflow: OverflowPolicy,
    pub producer_may_block: bool,
    pub retain_codec_configuration: bool,
    pub restart_requires_keyframe: bool,
    pub slow_consumer_action: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionBudget {
    pub bandwidth_bytes_per_second: u64,
    pub samples_or_frames_per_second: f64,
    pub decode_slots: u32,
    pub queue_bytes: u64,
    pub queue_duration_ms: u64,
    pub disk_bytes: u64,
    pub recorder_slots: u32,
    pub maximum_clock_uncertainty_ms: Option<f64>,
    pub priority_class: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentRun {
    pub run_id: String,
    pub protocol_id: String,
    pub protocol_version: String,
    pub participant_reference: Option<String>,
    #[serde(default)]
    pub required_stream_rules: Vec<String>,
    #[serde(default)]
    pub optional_stream_rules: Vec<String>,
    pub marker_schema: Option<String>,
    pub selection_snapshot_id: String,
    pub started_at_ms: i64,
    pub recording_policy_revision: u64,
    #[serde(default)]
    pub approved_deviations: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingArtifactState {
    Starting,
    Writing,
    Stalled,
    Finalizing,
    Complete,
    Failed,
    Cleaned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingArtifact {
    pub artifact_id: String,
    pub format: String,
    pub state: RecordingArtifactState,
    pub bytes_written: u64,
    pub last_write_ms: Option<i64>,
    #[serde(default)]
    pub native_metadata_digests: Vec<String>,
    pub clock_history_present: bool,
    pub checksum_sha256: Option<String>,
    pub encrypted_at_rest: bool,
    pub retention_until_ms: Option<i64>,
    pub cleanup_receipt_id: Option<String>,
    pub replay_validation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamDescriptor {
    pub schema: String,
    pub logical_stream_id: String,
    pub semantic: StreamSemantic,
    pub native_descriptor: NativeDescriptor,
    pub selection: SourceSelection,
    pub epochs: ComponentEpochs,
    pub accepted_authority_revision: u64,
    pub timing: TimingCorrelation,
    pub cadence: CadencePolicy,
    pub progress: ProgressProfile,
    #[serde(default)]
    pub queues: Vec<EdgeQueuePolicy>,
    pub budget: AdmissionBudget,
    pub experiment_run: Option<ExperimentRun>,
    pub recording: Option<RecordingArtifact>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl ValidateContract for StreamDescriptor {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.stream_descriptor.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.stream_descriptor.v1",
            ));
        }
        require_nonempty(&mut failures, &self.logical_stream_id, "logical_stream_id");
        require_nonempty(&mut failures, &self.semantic.family, "semantic.family");
        require_nonempty(&mut failures, &self.semantic.class, "semantic.class");
        require_nonempty(
            &mut failures,
            &self.semantic.schema_profile,
            "semantic.schema_profile",
        );
        require_nonempty(
            &mut failures,
            &self.native_descriptor.kind,
            "native_descriptor.kind",
        );
        if !is_sha256(&self.native_descriptor.digest_sha256) {
            failures.push(ContractViolation::new(
                "invalid_digest",
                "native_descriptor.digest_sha256",
                "native descriptor digest must be lowercase SHA-256",
            ));
        }
        if self.native_descriptor.document.is_some() == self.native_descriptor.reference.is_some() {
            failures.push(ContractViolation::new(
                "invalid_native_descriptor_storage",
                "native_descriptor",
                "exactly one native document or role-controlled reference is required",
            ));
        }
        require_nonempty(
            &mut failures,
            &self.selection.query_language,
            "selection.query_language",
        );
        require_nonempty(&mut failures, &self.selection.query, "selection.query");
        if self.selection.expected_cardinality == 0
            || self.selection.candidate_count != self.selection.candidate_descriptor_digests.len()
        {
            failures.push(ContractViolation::new(
                "invalid_selection_cardinality",
                "selection",
                "expected cardinality must be nonzero and candidate count must match evidence",
            ));
        }
        if self
            .selection
            .candidate_descriptor_digests
            .iter()
            .any(|digest| !is_sha256(digest))
        {
            failures.push(ContractViolation::new(
                "invalid_digest",
                "selection.candidate_descriptor_digests",
                "candidate digests must be lowercase SHA-256",
            ));
        }
        let selection_is_resolved = matches!(
            self.selection.method,
            SelectionMethod::SavedRule
                | SelectionMethod::ManualPin
                | SelectionMethod::OwnerManifestExactMatch
                | SelectionMethod::SingleCandidate
                | SelectionMethod::PolicyTiebreak
        );
        if selection_is_resolved != self.selection.chosen_native_instance.is_some() {
            failures.push(ContractViolation::new(
                "invalid_selection_result",
                "selection.chosen_native_instance",
                "resolved selection requires one chosen instance and unresolved selection forbids it",
            ));
        }
        if self.selection.method == SelectionMethod::ManualPin
            && (self.selection.override_actor.is_none() || self.selection.override_reason.is_none())
        {
            failures.push(ContractViolation::new(
                "missing_override_lineage",
                "selection",
                "manual pin requires actor and reason",
            ));
        }
        for (name, epoch) in [
            ("source", Some(&self.epochs.source)),
            ("route", self.epochs.route.as_ref()),
            ("processing", self.epochs.processing.as_ref()),
            ("sink", self.epochs.sink.as_ref()),
        ] {
            if let Some(epoch) = epoch {
                require_nonempty(&mut failures, &epoch.id, &format!("epochs.{name}.id"));
                require_nonempty(
                    &mut failures,
                    &epoch.reason,
                    &format!("epochs.{name}.reason"),
                );
            }
        }
        require_nonempty(
            &mut failures,
            &self.epochs.path_generation,
            "epochs.path_generation",
        );
        if self.accepted_authority_revision == 0 {
            failures.push(ContractViolation::new(
                "invalid_revision",
                "accepted_authority_revision",
                "accepted authority revision must be greater than zero",
            ));
        }
        for (index, domain) in self.timing.domains.iter().enumerate() {
            require_nonempty(
                &mut failures,
                &domain.domain_id,
                &format!("timing.domains[{index}].domain_id"),
            );
            if domain.time_base_denominator <= 0 || domain.time_base_numerator <= 0 {
                failures.push(ContractViolation::new(
                    "invalid_time_base",
                    &format!("timing.domains[{index}]"),
                    "time base values must be positive",
                ));
            }
        }
        let domain_ids = self
            .timing
            .domains
            .iter()
            .map(|domain| domain.domain_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if domain_ids.len() != self.timing.domains.len() {
            failures.push(ContractViolation::new(
                "duplicate_time_domain",
                "timing.domains",
                "time-domain IDs must be unique",
            ));
        }
        for (index, transform) in self.timing.transforms.iter().enumerate() {
            require_nonempty(
                &mut failures,
                &transform.transform_id,
                &format!("timing.transforms[{index}].transform_id"),
            );
            if !domain_ids.contains(transform.from_domain.as_str())
                || !domain_ids.contains(transform.to_domain.as_str())
                || !transform.uncertainty_ms.is_finite()
                || transform.uncertainty_ms < 0.0
                || !transform.offset_ms.is_finite()
                || transform
                    .valid_to_ms
                    .is_some_and(|valid_to| valid_to < transform.valid_from_ms)
            {
                failures.push(ContractViolation::new(
                    "invalid_time_transform",
                    &format!("timing.transforms[{index}]"),
                    "transform domains, finite values, uncertainty, and validity interval must be valid",
                ));
            }
        }
        if self.timing.fixed_latency_ms.is_some()
            != self.timing.fixed_latency_uncertainty_ms.is_some()
            || self.timing.fixed_latency_ms.is_some() && self.timing.calibration_reference.is_none()
        {
            failures.push(ContractViolation::new(
                "incomplete_latency_calibration",
                "timing",
                "fixed latency requires uncertainty and a calibration reference",
            ));
        }
        if self.cadence.measurement_window_ms == 0
            || matches!(self.cadence.mode, CadenceMode::Regular)
                && self.cadence.nominal_rate_hz.is_none()
        {
            failures.push(ContractViolation::new(
                "invalid_cadence",
                "cadence",
                "cadence needs a measurement window and regular streams need a nominal rate",
            ));
        }
        if let (Some(minimum), Some(maximum)) = (
            self.cadence.accepted_rate_min_hz,
            self.cadence.accepted_rate_max_hz,
        ) && (!minimum.is_finite() || !maximum.is_finite() || minimum < 0.0 || maximum < minimum)
        {
            failures.push(ContractViolation::new(
                "invalid_rate_range",
                "cadence",
                "accepted cadence bounds must be finite, nonnegative, and ordered",
            ));
        }
        if self.cadence.mode != CadenceMode::Regular
            && self.cadence.no_data_deadline_ms.is_none()
            && self.cadence.heartbeat.is_some()
        {
            failures.push(ContractViolation::new(
                "heartbeat_without_deadline",
                "cadence",
                "a heartbeat requires an explicit no-data deadline",
            ));
        }
        require_nonempty(
            &mut failures,
            &self.progress.profile_id,
            "progress.profile_id",
        );
        for (stage, evidence) in &self.progress.stages {
            if evidence.applicability == ProgressApplicability::Required
                && (evidence.deadline_ms.is_none() || evidence.state.is_none())
            {
                failures.push(ContractViolation::new(
                    "incomplete_required_stage",
                    &format!("progress.stages.{stage:?}"),
                    "required progress stage needs deadline and state",
                ));
            }
            if evidence.applicability == ProgressApplicability::NotApplicable
                && (evidence.deadline_ms.is_some() || evidence.state.is_some())
            {
                failures.push(ContractViolation::new(
                    "invalid_not_applicable_stage",
                    &format!("progress.stages.{stage:?}"),
                    "not-applicable progress stage must not report deadline or state",
                ));
            }
        }
        let mut queue_ids = BTreeMap::new();
        for (index, queue) in self.queues.iter().enumerate() {
            require_nonempty(
                &mut failures,
                &queue.edge_id,
                &format!("queues[{index}].edge_id"),
            );
            if queue.limits.items == 0 || queue.limits.bytes == 0 || queue.limits.duration_ms == 0 {
                failures.push(ContractViolation::new(
                    "unbounded_queue",
                    &format!("queues[{index}].limits"),
                    "every edge queue requires finite item, byte, and time limits",
                ));
            }
            require_nonempty(
                &mut failures,
                &queue.producer,
                &format!("queues[{index}].producer"),
            );
            require_nonempty(
                &mut failures,
                &queue.consumer,
                &format!("queues[{index}].consumer"),
            );
            require_nonempty(
                &mut failures,
                &queue.slow_consumer_action,
                &format!("queues[{index}].slow_consumer_action"),
            );
            if queue.overflow == OverflowPolicy::BlockProducer && !queue.producer_may_block {
                failures.push(ContractViolation::new(
                    "unsafe_block_policy",
                    &format!("queues[{index}]"),
                    "block-producer overflow requires explicit producer blocking support",
                ));
            }
            if queue_ids.insert(&queue.edge_id, index).is_some() {
                failures.push(ContractViolation::new(
                    "duplicate_queue",
                    &format!("queues[{index}].edge_id"),
                    "edge queue IDs must be unique",
                ));
            }
        }
        require_nonempty(
            &mut failures,
            &self.budget.priority_class,
            "budget.priority_class",
        );
        if self.budget.queue_bytes == 0 || self.budget.queue_duration_ms == 0 {
            failures.push(ContractViolation::new(
                "unbounded_budget",
                "budget",
                "admission budget must bound queue bytes and duration",
            ));
        }
        if let Some(recording) = &self.recording {
            require_nonempty(
                &mut failures,
                &recording.artifact_id,
                "recording.artifact_id",
            );
            require_nonempty(&mut failures, &recording.format, "recording.format");
            if recording
                .checksum_sha256
                .as_ref()
                .is_some_and(|digest| !is_sha256(digest))
            {
                failures.push(ContractViolation::new(
                    "invalid_digest",
                    "recording.checksum_sha256",
                    "recording checksum must be lowercase SHA-256",
                ));
            }
        }
        if self.recording.is_some() && self.experiment_run.is_none() {
            failures.push(ContractViolation::new(
                "recording_without_run",
                "recording",
                "scientific recording requires an experiment-run provenance record",
            ));
        }
        if let Some(run) = &self.experiment_run {
            require_nonempty(&mut failures, &run.run_id, "experiment_run.run_id");
            require_nonempty(
                &mut failures,
                &run.protocol_id,
                "experiment_run.protocol_id",
            );
            require_nonempty(
                &mut failures,
                &run.protocol_version,
                "experiment_run.protocol_version",
            );
            require_nonempty(
                &mut failures,
                &run.selection_snapshot_id,
                "experiment_run.selection_snapshot_id",
            );
            if run.recording_policy_revision == 0 {
                failures.push(ContractViolation::new(
                    "invalid_revision",
                    "experiment_run.recording_policy_revision",
                    "recording policy revision must be greater than zero",
                ));
            }
        }
        finish(failures)
    }
}
