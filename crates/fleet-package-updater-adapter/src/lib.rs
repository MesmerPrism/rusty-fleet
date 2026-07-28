// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded Fleet-side adapter for the Rusty Quest attended package updater.
//!
//! The adapter deliberately owns no Android, downloader, signature-key, or
//! PackageInstaller behavior. It prepares an exact owner invocation and can
//! validate the shape and binding of untrusted evidence before an authenticated
//! owner transport exists. Validation is not admission: the Hub cannot advance
//! owner-only lifecycle states through this adapter. An invocation
//! acknowledgement is transport evidence; only an authenticated effective
//! `install_commit` receipt can prove application.

use fleet_contracts::{
    PackageReleaseReference, PackageUpdaterEffectiveReceipt, PackageUpdaterInvocation,
    PackageUpdaterInvocationAcknowledgement, ValidateContract,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageUpdaterAdapterLimits {
    pub maximum_operation_lifetime_ms: i64,
    pub maximum_release_reference_bytes: usize,
}

impl Default for PackageUpdaterAdapterLimits {
    fn default() -> Self {
        Self {
            maximum_operation_lifetime_ms: 15 * 60 * 1_000,
            maximum_release_reference_bytes: 2_048,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageUpdaterAdapterError {
    InvalidLimits,
    ContractInvalid,
    Expired,
    BindingMismatch,
    ReleaseReferenceTooLarge,
    InvocationRejected,
    InstalledVersionNotProven,
}

impl std::fmt::Display for PackageUpdaterAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for PackageUpdaterAdapterError {}

#[derive(Clone, Debug)]
pub struct PackageUpdaterOwnerAdapter {
    limits: PackageUpdaterAdapterLimits,
}

impl PackageUpdaterOwnerAdapter {
    pub fn new(limits: PackageUpdaterAdapterLimits) -> Result<Self, PackageUpdaterAdapterError> {
        if limits.maximum_operation_lifetime_ms <= 0
            || limits.maximum_operation_lifetime_ms > 24 * 60 * 60 * 1_000
            || limits.maximum_release_reference_bytes == 0
            || limits.maximum_release_reference_bytes > 16 * 1_024
        {
            return Err(PackageUpdaterAdapterError::InvalidLimits);
        }
        Ok(Self { limits })
    }

    pub fn prepare_invocation(
        &self,
        invocation: PackageUpdaterInvocation,
        now_ms: i64,
    ) -> Result<PackageUpdaterInvocation, PackageUpdaterAdapterError> {
        invocation
            .validate()
            .map_err(|_| PackageUpdaterAdapterError::ContractInvalid)?;
        if now_ms < 0
            || now_ms >= invocation.expires_at_ms
            || invocation.expires_at_ms.saturating_sub(now_ms)
                > self.limits.maximum_operation_lifetime_ms
        {
            return Err(PackageUpdaterAdapterError::Expired);
        }
        if release_reference_len(&invocation.release) > self.limits.maximum_release_reference_bytes
        {
            return Err(PackageUpdaterAdapterError::ReleaseReferenceTooLarge);
        }
        Ok(invocation)
    }

    pub fn validate_untrusted_acknowledgement(
        &self,
        invocation: &PackageUpdaterInvocation,
        acknowledgement: PackageUpdaterInvocationAcknowledgement,
        now_ms: i64,
    ) -> Result<PackageUpdaterInvocationAcknowledgement, PackageUpdaterAdapterError> {
        acknowledgement
            .validate()
            .map_err(|_| PackageUpdaterAdapterError::ContractInvalid)?;
        if now_ms < acknowledgement.acknowledged_at_ms || now_ms > invocation.expires_at_ms {
            return Err(PackageUpdaterAdapterError::Expired);
        }
        if acknowledgement.operation_id != invocation.operation_id
            || acknowledgement.device_id != invocation.device_id
            || acknowledgement.owner_action_request_id != invocation.owner_action_request_id
        {
            return Err(PackageUpdaterAdapterError::BindingMismatch);
        }
        if !acknowledgement.accepted {
            return Err(PackageUpdaterAdapterError::InvocationRejected);
        }
        Ok(acknowledgement)
    }

    pub fn validate_untrusted_effective_receipt(
        &self,
        invocation: &PackageUpdaterInvocation,
        receipt: PackageUpdaterEffectiveReceipt,
        now_ms: i64,
    ) -> Result<PackageUpdaterEffectiveReceipt, PackageUpdaterAdapterError> {
        if now_ms < receipt.wrapped_at_ms || now_ms > invocation.expires_at_ms {
            return Err(PackageUpdaterAdapterError::Expired);
        }
        receipt.validate_for(invocation).map_err(|failures| {
            if failures
                .iter()
                .any(|failure| failure.code == "installed_version_not_proven")
            {
                PackageUpdaterAdapterError::InstalledVersionNotProven
            } else {
                PackageUpdaterAdapterError::BindingMismatch
            }
        })?;
        Ok(receipt)
    }
}

fn release_reference_len(reference: &PackageReleaseReference) -> usize {
    match reference {
        PackageReleaseReference::ManifestUrl { manifest_url } => manifest_url.len(),
        PackageReleaseReference::ReleaseId { release_id } => release_id.len(),
    }
}

#[cfg(test)]
mod tests {
    use fleet_contracts::{
        PACKAGE_UPDATE_RECEIPT_SCHEMA, PACKAGE_UPDATER_ACK_SCHEMA, PackageReleaseReference,
        PackageUpdateCheckpoint, PackageUpdateReceipt, PackageUpdateReceiptDecision,
        PackageUpdateReceiptStage, PackageUpdaterEffectiveReceipt, PackageUpdaterInvocation,
        PackageUpdaterInvocationAcknowledgement,
    };

    use super::{
        PackageUpdaterAdapterError, PackageUpdaterAdapterLimits, PackageUpdaterOwnerAdapter,
    };

    fn invocation() -> PackageUpdaterInvocation {
        PackageUpdaterInvocation {
            schema: "rusty.fleet.package_updater_invocation.v1".to_owned(),
            operation_id: "package-operation-1".to_owned(),
            preview_id: "package-preview-1".to_owned(),
            device_id: "device.quest.1".to_owned(),
            identity_revision: 7,
            owner_action_request_id: "package-owner-1".to_owned(),
            release: PackageReleaseReference::ManifestUrl {
                manifest_url: "https://updates.example.invalid/stable/manifest.json".to_owned(),
            },
            expected_package_name: "org.example.kiosk".to_owned(),
            expected_rollout_ring: "stable".to_owned(),
            expires_at_ms: 10_000,
        }
    }

    fn effective_receipt() -> PackageUpdaterEffectiveReceipt {
        let digest = format!("sha256:{}", "a".repeat(64));
        PackageUpdaterEffectiveReceipt {
            schema: "rusty.fleet.package_updater_effective_receipt.v1".to_owned(),
            operation_id: "package-operation-1".to_owned(),
            device_id: "device.quest.1".to_owned(),
            identity_revision: 7,
            owner_action_request_id: "package-owner-1".to_owned(),
            updater_receipt: PackageUpdateReceipt {
                schema: PACKAGE_UPDATE_RECEIPT_SCHEMA.to_owned(),
                stage: PackageUpdateReceiptStage::InstallCommit,
                decision: PackageUpdateReceiptDecision::Accepted,
                code: "installed".to_owned(),
                observed_at_ms: 3_000,
                envelope_sha256: Some(format!("sha256:{}", "b".repeat(64))),
                signed_manifest_sha256: Some(digest.clone()),
                key_id: Some("release-key-1".to_owned()),
                manifest_id: Some("release-15".to_owned()),
                package_name: Some("org.example.kiosk".to_owned()),
                rollout_ring: Some("stable".to_owned()),
                sequence: Some(15),
                version_code: Some(15),
                prior_checkpoint: None,
                accepted_checkpoint: Some(PackageUpdateCheckpoint {
                    package_name: "org.example.kiosk".to_owned(),
                    rollout_ring: "stable".to_owned(),
                    sequence: 15,
                    version_code: 15,
                    signed_manifest_sha256: digest,
                }),
                state_changed: true,
            },
            wrapped_at_ms: 3_000,
        }
    }

    #[test]
    fn acknowledgement_never_substitutes_for_application_proof() {
        let adapter = PackageUpdaterOwnerAdapter::new(PackageUpdaterAdapterLimits::default())
            .expect("valid limits");
        let invocation = adapter
            .prepare_invocation(invocation(), 2_000)
            .expect("valid invocation");
        let acknowledgement = PackageUpdaterInvocationAcknowledgement {
            schema: PACKAGE_UPDATER_ACK_SCHEMA.to_owned(),
            operation_id: invocation.operation_id.clone(),
            device_id: invocation.device_id.clone(),
            owner_action_request_id: invocation.owner_action_request_id.clone(),
            accepted: true,
            code: "accepted".to_owned(),
            acknowledged_at_ms: 2_001,
        };
        assert!(
            adapter
                .validate_untrusted_acknowledgement(&invocation, acknowledgement, 2_002)
                .is_ok()
        );
        assert!(
            adapter
                .validate_untrusted_effective_receipt(&invocation, effective_receipt(), 3_001)
                .is_ok()
        );
    }

    #[test]
    fn manifest_admission_and_wrong_package_do_not_prove_install() {
        let adapter = PackageUpdaterOwnerAdapter::new(PackageUpdaterAdapterLimits::default())
            .expect("valid limits");
        let invocation = invocation();
        let mut receipt = effective_receipt();
        receipt.updater_receipt.stage = PackageUpdateReceiptStage::ManifestAdmission;
        assert_eq!(
            adapter
                .validate_untrusted_effective_receipt(&invocation, receipt, 3_001)
                .expect_err("manifest admission is not installed readback"),
            PackageUpdaterAdapterError::InstalledVersionNotProven
        );
        let mut receipt = effective_receipt();
        receipt.updater_receipt.package_name = Some("org.example.other".to_owned());
        assert_eq!(
            adapter
                .validate_untrusted_effective_receipt(&invocation, receipt, 3_001)
                .expect_err("wrong package is not application"),
            PackageUpdaterAdapterError::InstalledVersionNotProven
        );
    }
}
