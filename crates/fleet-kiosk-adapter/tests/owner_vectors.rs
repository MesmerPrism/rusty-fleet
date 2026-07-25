// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use fleet_kiosk_adapter::{
    AdapterError, HEADER_CONTENT_SHA256, HEADER_REQUEST_ID, HEADER_SIGNATURE, KioskHttpResponse,
    sha256_hex, sign_request, sign_response, verify_signed_response,
};

const KEY: &str = "0123-4567-89AB-CDEF";
const REQUEST_ID: &str = "http_12345678";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn owner_vector_response() -> Result<KioskHttpResponse, AdapterError> {
    Ok(KioskHttpResponse {
        status: 200,
        headers: BTreeMap::from([
            (HEADER_REQUEST_ID.to_owned(), REQUEST_ID.to_owned()),
            (HEADER_CONTENT_SHA256.to_owned(), EMPTY_SHA256.to_owned()),
            (
                HEADER_SIGNATURE.to_owned(),
                sign_response(KEY, REQUEST_ID, 200, EMPTY_SHA256)?,
            ),
        ]),
        body: Vec::new(),
        redirected: false,
        received_at_ms: 1_784_650_000_000,
    })
}

#[test]
fn matches_kiosk_owner_request_and_response_vectors() -> Result<(), AdapterError> {
    assert_eq!(sha256_hex(b""), EMPTY_SHA256);
    assert_eq!(
        sign_request(
            KEY,
            "POST",
            "/v1/kiosk/invoke",
            REQUEST_ID,
            1_784_650_000,
            EMPTY_SHA256,
        )?,
        "f35ef975435590bf944f26e5055267d3615c6f1916f4a8b3986389900b588989"
    );
    assert_eq!(
        sign_response(KEY, REQUEST_ID, 200, EMPTY_SHA256)?,
        "0a4418fe4677bfac1a12047ef8ea842e3ebaca7e758b8a190a4de009eaf9babb"
    );
    verify_signed_response(KEY, REQUEST_ID, &owner_vector_response()?)
}

#[test]
fn rejects_damaged_signed_response_envelopes() -> Result<(), AdapterError> {
    let mut missing_signature = owner_vector_response()?;
    missing_signature.headers.remove(HEADER_SIGNATURE);
    assert_eq!(
        verify_signed_response(KEY, REQUEST_ID, &missing_signature),
        Err(AdapterError::MissingResponseHeader(HEADER_SIGNATURE))
    );

    let mut substituted_transport_id = owner_vector_response()?;
    substituted_transport_id
        .headers
        .insert(HEADER_REQUEST_ID.to_owned(), "other_12345678".to_owned());
    assert_eq!(
        verify_signed_response(KEY, REQUEST_ID, &substituted_transport_id),
        Err(AdapterError::ResponseRequestIdMismatch)
    );

    let mut tampered_body = owner_vector_response()?;
    tampered_body.body = b"{}".to_vec();
    assert_eq!(
        verify_signed_response(KEY, REQUEST_ID, &tampered_body),
        Err(AdapterError::ResponseDigestMismatch)
    );

    let mut tampered_signature = owner_vector_response()?;
    tampered_signature.headers.insert(
        HEADER_SIGNATURE.to_owned(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
    );
    assert_eq!(
        verify_signed_response(KEY, REQUEST_ID, &tampered_signature),
        Err(AdapterError::ResponseSignatureMismatch)
    );
    Ok(())
}
