use surfacecheck_core::{
    from_json, to_canonical_json, AgentCategory, AgentFinding, AgentReviewResponse, BeforeAfter,
    CaptureRecord, CaptureType, CliResponse, ContractError, DeterministicCategory,
    DeterministicFinding, Dimensions, EmptyRequest, ErrorCode, ErrorEnvelope, EvidenceManifest,
    EvidenceRef, EvidenceRegion, FindingSource, ImageEvidence, OperationStatus, Provenance,
    ProvenanceKind, Scale, Severity, ToolName, ToolVersion, Validate, MAX_AGENT_PROMPT_BYTES,
    MAX_IMAGE_BYTES, MAX_JSON_FRAME_BYTES, SCHEMA_VERSION,
};

const VALID_MANIFEST: &str = include_str!("fixtures/valid_manifest.json");

fn valid_provenance(kind: ProvenanceKind) -> Provenance {
    Provenance {
        kind,
        producer: "surfacecheck-test".to_owned(),
        producer_version: "0.1.0".to_owned(),
        producer_commit: "test-commit".to_owned(),
        tool_versions: vec![ToolVersion {
            name: ToolName::Surfacecheck,
            version: "0.1.0".to_owned(),
        }],
    }
}

fn valid_capture(id: &str) -> CaptureRecord {
    CaptureRecord {
        capture_id: id.to_owned(),
        capture_type: CaptureType::Window,
        captured_at: 1_700_000_000_000,
        image: ImageEvidence {
            relative_path: format!("captures/{id}.png"),
            bytes: 128,
            sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        },
        dimensions: Dimensions {
            width: 800,
            height: 600,
        },
        scale: Scale { x: 1.25, y: 1.25 },
        selection: None,
        tool_versions: vec![ToolVersion {
            name: ToolName::Grim,
            version: "1.5.0".to_owned(),
        }],
        application: None,
    }
}

fn valid_evidence(capture_id: &str) -> EvidenceRef {
    EvidenceRef {
        capture_id: capture_id.to_owned(),
        content_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        region: EvidenceRegion {
            x: 10,
            y: 20,
            width: 100,
            height: 80,
        },
    }
}

fn valid_agent_finding(capture_id: &str) -> AgentFinding {
    AgentFinding {
        finding_id: "agent-1".to_owned(),
        category: AgentCategory::Layout,
        severity: Severity::Medium,
        evidence: vec![valid_evidence(capture_id)],
        explanation: "The selected panel is visibly misaligned.".to_owned(),
        confidence: 0.75,
        suggested_next_action: "Inspect the panel alignment rules.".to_owned(),
    }
}

fn valid_manifest() -> EvidenceManifest {
    EvidenceManifest {
        schema_version: SCHEMA_VERSION,
        session_id: "session-1".to_owned(),
        created_at: 1_700_000_000_000,
        captures: vec![valid_capture("capture-1")],
        user_note: Some("Check the panel spacing".to_owned()),
        deterministic_findings: vec![DeterministicFinding {
            finding_id: "det-1".to_owned(),
            category: DeterministicCategory::ContrastMeasurement,
            severity: Severity::Info,
            evidence: vec![valid_evidence("capture-1")],
            explanation: "Measured luminance range is recorded for review.".to_owned(),
            code: "contrast_measurement".to_owned(),
            measurement: Some(0.42),
        }],
        agent_findings: vec![valid_agent_finding("capture-1")],
        comparison: None,
        before_after: None,
        provenance: valid_provenance(ProvenanceKind::LocalCapture),
    }
}

#[test]
fn valid_fixture_round_trips_and_is_canonical() {
    let manifest: EvidenceManifest =
        from_json(VALID_MANIFEST.as_bytes()).expect("fixture is valid");
    let first = to_canonical_json(&manifest).expect("manifest validates");
    let second = to_canonical_json(&manifest).expect("manifest validates twice");
    assert_eq!(
        first, second,
        "controlled values must serialize byte-identically"
    );

    let reparsed: EvidenceManifest = from_json(&first).expect("canonical output parses");
    assert_eq!(manifest, reparsed);
    assert!(first.starts_with(b"{\"schemaVersion\""));
}

#[test]
fn unknown_fields_are_rejected() {
    let input = br#"{
        "schemaVersion": 1,
        "sessionId": "session-1",
        "createdAt": 1700000000000,
        "captures": [],
        "userNote": null,
        "deterministicFindings": [],
        "agentFindings": [],
        "comparison": null,
        "beforeAfter": null,
        "provenance": {
            "kind": "local_capture",
            "producer": "test",
            "producerVersion": "0.1",
            "producerCommit": "abc",
            "toolVersions": []
        },
        "unexpected": true
    }"#;
    assert!(matches!(
        from_json::<EvidenceManifest>(input),
        Err(ContractError::Json(_))
    ));
}

#[test]
fn malformed_enum_is_rejected() {
    let input = VALID_MANIFEST.replace("\"window\"", "\"desktop\"");
    assert!(matches!(
        from_json::<EvidenceManifest>(input.as_bytes()),
        Err(ContractError::Json(_))
    ));
}

#[test]
fn schema_version_and_bounds_are_enforced() {
    let mut manifest = valid_manifest();
    manifest.schema_version = 2;
    assert!(manifest.validate().is_err());

    let mut capture = valid_capture("too-large");
    capture.image.bytes = MAX_IMAGE_BYTES + 1;
    assert!(capture.validate().is_err());

    let mut huge_note = valid_manifest();
    huge_note.user_note = Some("x".repeat(16 * 1024 + 1));
    assert!(huge_note.validate().is_err());
}

#[test]
fn non_finite_agent_confidence_is_rejected() {
    let mut finding = valid_agent_finding("capture-1");
    finding.confidence = f64::NAN;
    assert!(finding.validate().is_err());
    finding.confidence = f64::INFINITY;
    assert!(finding.validate().is_err());
}

#[test]
fn evidence_coordinates_must_fit_the_capture() {
    let mut manifest = valid_manifest();
    manifest.agent_findings[0].evidence[0].region.x = 799;
    assert!(manifest.validate().is_err());
}

#[test]
fn duplicate_captures_are_rejected() {
    let mut manifest = valid_manifest();
    manifest.captures.push(valid_capture("capture-1"));
    assert!(manifest.validate().is_err());
}

#[test]
fn response_statuses_have_honest_shapes() {
    let success = CliResponse {
        schema_version: SCHEMA_VERSION,
        request_id: "request-1".to_owned(),
        status: OperationStatus::Success,
        result: Some(EmptyRequest {}),
        error: None,
    };
    assert!(success.validate().is_ok());

    let dishonest = CliResponse {
        schema_version: SCHEMA_VERSION,
        request_id: "request-1".to_owned(),
        status: OperationStatus::Busy,
        result: Some(EmptyRequest {}),
        error: None,
    };
    assert!(dishonest.validate().is_err());

    let unavailable = CliResponse::<EmptyRequest> {
        schema_version: SCHEMA_VERSION,
        request_id: "request-1".to_owned(),
        status: OperationStatus::Unavailable,
        result: None,
        error: Some(ErrorEnvelope {
            code: ErrorCode::RuntimeUnavailable,
            message: "No compositor is available".to_owned(),
            retryable: true,
        }),
    };
    assert!(unavailable.validate().is_ok());
}

#[test]
fn agent_response_keeps_findings_separate_and_bounded() {
    let response = AgentReviewResponse {
        schema_version: SCHEMA_VERSION,
        review_id: "review-1".to_owned(),
        status: OperationStatus::Success,
        findings: vec![valid_agent_finding("capture-1")],
        error: None,
        provenance: valid_provenance(ProvenanceKind::AgentReview),
    };
    assert!(response.validate().is_ok());

    let too_many = AgentReviewResponse {
        findings: vec![valid_agent_finding("capture-1"); 129],
        ..response
    };
    assert!(too_many.validate().is_err());
}

#[test]
fn framing_and_prompt_limits_are_explicit() {
    assert!(from_json::<EvidenceManifest>(&vec![b' '; MAX_JSON_FRAME_BYTES + 1]).is_err());
    let mut request = surfacecheck_core::AgentReviewRequest {
        schema_version: SCHEMA_VERSION,
        review_id: "review-1".to_owned(),
        capture_id: "capture-1".to_owned(),
        prompt: "x".repeat(MAX_AGENT_PROMPT_BYTES + 1),
        evidence: vec![valid_evidence("capture-1")],
        provenance: valid_provenance(ProvenanceKind::AgentReview),
    };
    assert!(request.validate().is_err());
    request.prompt = "review".to_owned();
    assert!(request.validate().is_ok());
}

#[test]
fn comparison_and_before_after_references_are_checked() {
    let mut manifest = valid_manifest();
    manifest.captures.push(valid_capture("capture-2"));
    manifest.comparison = Some(surfacecheck_core::ComparisonRecord {
        comparison_id: "comparison-1".to_owned(),
        before_capture_id: "capture-1".to_owned(),
        after_capture_id: "capture-2".to_owned(),
        dimensions: Dimensions {
            width: 800,
            height: 600,
        },
        same_scale: true,
        changed_pixels: 10,
        changed_fraction: 10.0 / 480_000.0,
        mean_absolute_difference: 0.1,
        rms_difference: 0.2,
        perceptual_distance: Some(0.2),
    });
    manifest.before_after = Some(BeforeAfter {
        before_capture_id: "capture-1".to_owned(),
        after_capture_id: "capture-2".to_owned(),
        comparison_id: "comparison-1".to_owned(),
    });
    assert!(manifest.validate().is_ok());

    manifest
        .before_after
        .as_mut()
        .expect("set above")
        .comparison_id = "missing".to_owned();
    assert!(manifest.validate().is_err());
}

#[test]
fn finding_source_is_closed_and_not_implicitly_agent_authority() {
    let source = FindingSource::Deterministic;
    let json = serde_json::to_string(&source).expect("enum serializes");
    assert_eq!(json, "\"deterministic\"");
}
