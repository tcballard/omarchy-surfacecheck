use surfacecheck_core::{
    from_json, to_canonical_json, Dimensions, EvidenceManifest, Scale, Validate,
    MAX_JSON_FRAME_BYTES, SCHEMA_VERSION,
};

#[test]
fn malformed_and_oversized_json_fail_before_deserialization() {
    assert!(from_json::<EvidenceManifest>(b"{").is_err());
    assert!(from_json::<EvidenceManifest>(&vec![b' '; MAX_JSON_FRAME_BYTES + 1]).is_err());
}

#[test]
fn fractional_scaling_and_extreme_dimensions_are_checked() {
    assert!(Scale { x: 1.25, y: 1.5 }.validate().is_ok());
    assert!(Scale {
        x: f64::NAN,
        y: 1.0
    }
    .validate()
    .is_err());
    assert!(Dimensions {
        width: 16_384,
        height: 16_384
    }
    .validate()
    .is_err());
}

#[test]
fn canonical_contract_bytes_are_reproducible_for_controlled_inputs() {
    let first: surfacecheck_core::EmptyRequest = serde_json::from_slice(b"{}").expect("request");
    assert_eq!(first.validate(), Ok(()));
    let encoded_a = to_canonical_json(&first).expect("canonical");
    let encoded_b = to_canonical_json(&first).expect("canonical");
    assert_eq!(encoded_a, encoded_b);
    assert_eq!(SCHEMA_VERSION, 1);
}
