//! Foreground service implementation for the v1 local protocol.

use crate::{
    authenticate_peer, read_frame, write_frame, RuntimePaths, ServiceError, SingleFlight,
    MAX_CLIENTS,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use surfacecheck_capture::{CaptureEngine, CaptureMode, DirectCommandRunner};
use surfacecheck_core::{
    from_json, json_nesting_within_limit, to_canonical_json, AgentFinding, AgentReviewRequest,
    BeforeAfter, CaptureApplicationRequest, CaptureRecord, CaptureRegionRequest, CaptureType,
    CaptureWindowRequest, CliCommand, CompareRequest, ConsentRecord, DefectEnvelope,
    DeterministicFinding, Dimensions, ErrorCode, ErrorEnvelope, EvidenceManifest, EvidenceRef,
    FindingSource, HandoffFindingRequest, ImageEvidence, OperationStatus,
    PremonitionHandoffRequest, Provenance, ProvenanceKind, Region, ReviewRequest, Scale,
    SelectBeforeAfterRequest, ToolVersion, Validate, MAX_FINDINGS, MAX_JSON_FRAME_BYTES,
    PREMONITION_PROTOCOL_VERSION, SCHEMA_VERSION,
};
use surfacecheck_review::{
    deterministic_findings, AgentReviewContext, CancellationToken, CaptureContext,
    PremonitionCoordinator, PremonitionHandoffError, ReviewCoordinator, ReviewInput,
};
use surfacecheck_store::{EvidenceStore, StoreConfig, StoreError};

const DEFAULT_SESSION_ID: &str = "session-current";
const PRODUCER_VERSION: &str = "0.1.0";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRequest {
    schema_version: u16,
    request_id: String,
    command: CliCommand,
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawResponse {
    schema_version: u16,
    request_id: String,
    status: OperationStatus,
    result: Option<serde_json::Value>,
    error: Option<ErrorEnvelope>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResult {
    daemon: OperationStatus,
    tools: Vec<surfacecheck_capture::ToolAvailability>,
    agent: OperationStatus,
    handoff: OperationStatus,
    max_captures: usize,
    max_bundle_bytes: u64,
    current_operation: Option<crate::BusyOperation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureResult {
    session_id: String,
    capture_id: String,
    capture_type: CaptureType,
    dimensions: Dimensions,
    scale: Scale,
    sha256: String,
    stale: bool,
    stored: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewResult {
    session_id: String,
    capture_id: String,
    deterministic_findings: Vec<DeterministicFinding>,
    agent_status: OperationStatus,
    agent_findings: Vec<AgentFinding>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportResult {
    session_id: String,
    relative_path: String,
    format: &'static str,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelResult {
    operation_id: String,
    generation: u64,
    cancelled: bool,
}

pub struct RuntimeService {
    store: EvidenceStore,
    flight: SingleFlight,
    agent: Option<Arc<dyn surfacecheck_review::AgentAdapter>>,
    premonition: Option<Arc<dyn surfacecheck_review::PremonitionAdapter>>,
}

impl RuntimeService {
    pub fn new(config: StoreConfig) -> Result<Self, ServiceError> {
        Ok(Self {
            store: EvidenceStore::new(config).map_err(store_error)?,
            flight: SingleFlight::default(),
            agent: None,
            premonition: None,
        })
    }

    pub fn from_environment() -> Result<Self, ServiceError> {
        let root = std::env::var_os("SURFACECHECK_STATE_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .map(|path| path.join("surfacecheck"))
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|path| path.join(".local/state/surfacecheck"))
            })
            .ok_or(ServiceError::InvalidRuntimePath)?;
        Self::new(StoreConfig::new(root))
    }

    pub fn with_adapters(
        mut self,
        agent: Option<Arc<dyn surfacecheck_review::AgentAdapter>>,
        premonition: Option<Arc<dyn surfacecheck_review::PremonitionAdapter>>,
    ) -> Self {
        self.agent = agent;
        self.premonition = premonition;
        self
    }

    pub fn handle_frame(&self, frame: &[u8]) -> Vec<u8> {
        if frame.is_empty() || frame.len() > MAX_JSON_FRAME_BYTES {
            return encode(&error_response(
                "invalid-request",
                OperationStatus::Invalid,
                ErrorCode::InvalidRequest,
                "request exceeds the bounded frame limit",
            ));
        }
        if !json_nesting_within_limit(frame) {
            return encode(&error_response(
                "invalid-request",
                OperationStatus::Invalid,
                ErrorCode::InvalidRequest,
                "request nesting exceeds the bounded limit",
            ));
        }
        let request = match serde_json::from_slice::<RawRequest>(frame) {
            Ok(request) => request,
            Err(_) => {
                return encode(&error_response(
                    "invalid-request",
                    OperationStatus::Invalid,
                    ErrorCode::InvalidRequest,
                    "request is malformed",
                ))
            }
        };
        if request.schema_version != SCHEMA_VERSION || !valid_id(&request.request_id) {
            let response_id = if valid_id(&request.request_id) {
                request.request_id.as_str()
            } else {
                "invalid-request"
            };
            return encode(&error_response(
                response_id,
                OperationStatus::Invalid,
                ErrorCode::InvalidRequest,
                "request version or ID is invalid",
            ));
        }
        let response = self.dispatch(request);
        encode(&response)
    }

    fn dispatch(&self, request: RawRequest) -> RawResponse {
        match request.command {
            CliCommand::Status => self.status_with_payload(request.request_id, request.payload),
            CliCommand::CaptureWindow => self.capture_window(request.request_id, request.payload),
            CliCommand::CaptureRegion => self.capture_region(request.request_id, request.payload),
            CliCommand::CaptureApplication => {
                self.capture_application(request.request_id, request.payload)
            }
            CliCommand::Review => self.review(request.request_id, request.payload),
            CliCommand::Compare => self.compare(request.request_id, request.payload),
            CliCommand::Annotate => self.annotate(request.request_id, request.payload),
            CliCommand::SelectBeforeAfter => {
                self.select_before_after(request.request_id, request.payload)
            }
            CliCommand::Export => self.export(request.request_id, request.payload),
            CliCommand::HandoffPremonition => self.handoff(request.request_id, request.payload),
            CliCommand::Cancel => self.cancel(request.request_id, request.payload),
            CliCommand::Service => self.status_with_payload(request.request_id, request.payload),
        }
    }

    fn status_with_payload(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        if serde_json::from_value::<surfacecheck_core::EmptyRequest>(payload).is_err() {
            return error_response(
                &request_id,
                OperationStatus::Invalid,
                ErrorCode::InvalidRequest,
                "status payload must be an empty object",
            );
        }
        self.status(request_id)
    }

    fn status(&self, request_id: String) -> RawResponse {
        let mut engine = CaptureEngine::new(DirectCommandRunner);
        engine.timeout = std::time::Duration::from_millis(500);
        let tools = engine.probe_tools();
        let result = StatusResult {
            daemon: OperationStatus::Success,
            tools,
            agent: if self.agent.is_some() {
                OperationStatus::Success
            } else {
                OperationStatus::Unavailable
            },
            handoff: if self.premonition.is_some() {
                OperationStatus::Success
            } else {
                OperationStatus::Unavailable
            },
            max_captures: self.store.config().max_captures,
            max_bundle_bytes: self.store.config().max_bundle_bytes,
            current_operation: self.flight.active(),
        };
        success(
            request_id,
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )
    }

    fn capture_window(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        match typed_payload::<CaptureWindowRequest>(payload) {
            Ok(request) => self.capture(
                request_id,
                request.session_id,
                request.user_note,
                None,
                None,
                CaptureMode::ActiveWindow,
            ),
            Err(response) => response_for_error(request_id, response),
        }
    }

    fn capture_region(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        match typed_payload::<CaptureRegionRequest>(payload) {
            Ok(request) => {
                let requested_region = request.region.clone();
                let mode = requested_region
                    .clone()
                    .map_or(CaptureMode::Region, |region| CaptureMode::ExplicitRegion {
                        region,
                    });
                self.capture(
                    request_id,
                    request.session_id,
                    request.user_note,
                    requested_region,
                    None,
                    mode,
                )
            }
            Err(response) => response_for_error(request_id, response),
        }
    }

    fn capture_application(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        match typed_payload::<CaptureApplicationRequest>(payload) {
            Ok(request) => self.capture(
                request_id,
                request.session_id,
                request.user_note,
                None,
                request.application_alias,
                CaptureMode::Application {
                    address: request.application_address,
                },
            ),
            Err(response) => response_for_error(request_id, response),
        }
    }

    fn capture(
        &self,
        request_id: String,
        session_id: Option<String>,
        note: Option<String>,
        requested_region: Option<Region>,
        application_alias: Option<String>,
        mode: CaptureMode,
    ) -> RawResponse {
        let lease = match self.flight.begin(&request_id) {
            Ok(lease) => lease,
            Err(_) => {
                return error_response(
                    &request_id,
                    OperationStatus::Busy,
                    ErrorCode::OperationBusy,
                    "another operation is active",
                )
            }
        };
        let mut engine =
            CaptureEngine::new(DirectCommandRunner).with_cancellation(lease.cancellation_flag());
        let outcome = engine.capture(mode);
        let response = match outcome {
            Ok(_) if lease.is_cancelled() => error_response(
                &request_id,
                OperationStatus::Cancelled,
                ErrorCode::SelectionCancelled,
                "capture was cancelled before evidence publication",
            ),
            Ok(outcome) => {
                let session_id = session_id.unwrap_or_else(|| DEFAULT_SESSION_ID.to_owned());
                match self.publish_capture(
                    &session_id,
                    note,
                    application_alias,
                    requested_region,
                    outcome,
                    &mut engine,
                ) {
                    Ok(result) => success(
                        request_id.clone(),
                        serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
                    ),
                    Err(error) => store_response(&request_id, error),
                }
            }
            Err(error) => error_response(
                &request_id,
                error.status,
                error.code,
                capture_message(error.status),
            ),
        };
        self.flight.finish(&lease);
        response
    }

    fn publish_capture(
        &self,
        session_id: &str,
        note: Option<String>,
        application_alias: Option<String>,
        requested_region: Option<Region>,
        outcome: surfacecheck_capture::CaptureOutcome,
        engine: &mut CaptureEngine<DirectCommandRunner>,
    ) -> Result<CaptureResult, StoreError> {
        match self.store.create_session(session_id) {
            Ok(_) | Err(StoreError::Exists(_)) => {}
            Err(error) => return Err(error),
        }
        let mut manifest = match self.store.read_manifest(session_id) {
            Ok(bytes) => from_json::<EvidenceManifest>(&bytes)
                .map_err(|error| StoreError::Manifest(error.to_string()))?,
            Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                empty_manifest(session_id)
            }
            Err(error) => return Err(error),
        };
        if manifest.captures.len() >= self.store.config().max_captures {
            return Err(StoreError::Quota("capture count limit reached".into()));
        }
        let capture_id = format!(
            "capture-{:02}-{}",
            manifest.captures.len() + 1,
            &outcome.image.sha256[..12]
        );
        let scale = capture_scale();
        let tool_versions = engine
            .probe_tools()
            .into_iter()
            .filter_map(|tool| {
                tool.version.map(|version| ToolVersion {
                    name: tool.tool,
                    version,
                })
            })
            .collect::<Vec<_>>();
        let region = requested_region.or_else(|| Some(outcome.region.clone()));
        let review_input = ReviewInput {
            capture_id: capture_id.clone(),
            image: outcome.image.clone(),
            scale: scale.clone(),
            stale: outcome.stale,
            duplicate_of: manifest
                .captures
                .iter()
                .find(|capture| capture.image.sha256 == outcome.image.sha256)
                .map(|capture| capture.capture_id.clone()),
            expected_dimensions: None,
        };
        let findings = deterministic_findings(&review_input)
            .map_err(|error| StoreError::Manifest(error.to_string()))?;
        let capture = CaptureRecord {
            capture_id: capture_id.clone(),
            capture_type: outcome.capture_type,
            captured_at: now_seconds(),
            stale: outcome.stale,
            image: ImageEvidence {
                relative_path: format!("captures/{capture_id}.png"),
                bytes: u64::try_from(outcome.png_bytes.len())
                    .map_err(|_| StoreError::Quota("image length overflows".into()))?,
                sha256: outcome.image.sha256.clone(),
            },
            dimensions: outcome.image.dimensions.clone(),
            scale: scale.clone(),
            selection: region,
            tool_versions: tool_versions.clone(),
            application: application_alias
                .and_then(|alias| redact_alias(&alias))
                .map(|redacted_alias| surfacecheck_core::ApplicationIdentity { redacted_alias }),
        };
        manifest.captures.push(capture);
        if let Some(note) = note {
            manifest.user_note = Some(note);
        }
        manifest.deterministic_findings.extend(findings);
        if manifest.deterministic_findings.len() > MAX_FINDINGS {
            return Err(StoreError::Quota("finding count limit reached".into()));
        }
        manifest.provenance = Provenance {
            kind: ProvenanceKind::LocalCapture,
            producer: "surfacecheckd".into(),
            producer_version: PRODUCER_VERSION.into(),
            producer_commit: producer_commit(),
            tool_versions,
        };
        let manifest_bytes = to_canonical_json(&manifest)
            .map_err(|error| StoreError::Manifest(error.to_string()))?;
        self.store
            .publish_capture(session_id, &capture_id, &outcome.png_bytes, &manifest_bytes)?;
        Ok(CaptureResult {
            session_id: session_id.to_owned(),
            capture_id,
            capture_type: outcome.capture_type,
            dimensions: outcome.image.dimensions,
            scale,
            sha256: outcome.image.sha256,
            stale: outcome.stale,
            stored: true,
        })
    }

    fn review(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let request = match typed_payload::<ReviewRequest>(payload) {
            Ok(request) => request,
            Err(response) => return response_for_error(request_id, response),
        };
        let session_id = request
            .session_id
            .clone()
            .unwrap_or_else(|| DEFAULT_SESSION_ID.to_owned());
        let lease = match self.flight.begin(&request_id) {
            Ok(lease) => lease,
            Err(_) => {
                return error_response(
                    &request_id,
                    OperationStatus::Busy,
                    ErrorCode::OperationBusy,
                    "another operation is active",
                )
            }
        };
        let response = match self.review_capture(&request_id, &session_id, &request, &lease) {
            Ok(result) => success(
                request_id.clone(),
                serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
            ),
            Err(error) => review_error_response(&request_id, error),
        };
        self.flight.finish(&lease);
        response
    }

    fn review_capture(
        &self,
        request_id: &str,
        session_id: &str,
        request: &ReviewRequest,
        lease: &crate::OperationLease,
    ) -> Result<ReviewResult, ReviewServiceError> {
        let manifest = self.read_manifest(session_id)?;
        let selected_capture_id = if request.capture_id == "latest" {
            manifest
                .captures
                .last()
                .map(|capture| capture.capture_id.clone())
                .ok_or(ReviewServiceError::NotFound)?
        } else {
            request.capture_id.clone()
        };
        let capture = manifest
            .captures
            .iter()
            .find(|capture| capture.capture_id == selected_capture_id)
            .ok_or(ReviewServiceError::NotFound)?
            .clone();
        let png = self.store.read_capture(session_id, &selected_capture_id)?;
        let image =
            surfacecheck_capture::decode_png(&png, surfacecheck_capture::PngLimits::default())
                .map_err(|_| ReviewServiceError::InvalidEvidence)?;
        let input = ReviewInput {
            capture_id: capture.capture_id.clone(),
            image: image.clone(),
            scale: capture.scale.clone(),
            stale: capture.stale,
            duplicate_of: None,
            expected_dimensions: Some(capture.dimensions.clone()),
        };
        let deterministic =
            deterministic_findings(&input).map_err(|_| ReviewServiceError::InvalidEvidence)?;
        let mut updated = manifest;
        updated.deterministic_findings.retain(|finding| {
            !finding
                .evidence
                .iter()
                .any(|evidence| evidence.capture_id == selected_capture_id)
        });
        updated.deterministic_findings.extend(deterministic.clone());
        updated.agent_findings.retain(|finding| {
            !finding
                .evidence
                .iter()
                .any(|evidence| evidence.capture_id == selected_capture_id)
        });
        let mut agent_status = OperationStatus::Unavailable;
        let mut agent_findings = Vec::new();
        if request.disclose_agent {
            if let Some(adapter) = &self.agent {
                let evidence = EvidenceRef {
                    capture_id: capture.capture_id.clone(),
                    content_sha256: capture.image.sha256.clone(),
                    region: surfacecheck_core::EvidenceRegion {
                        x: 0,
                        y: 0,
                        width: capture.dimensions.width,
                        height: capture.dimensions.height,
                    },
                };
                let agent_request = AgentReviewRequest {
                    schema_version: SCHEMA_VERSION,
                    review_id: format!("review-{request_id}"),
                    capture_id: capture.capture_id.clone(),
                    prompt: "Describe only visible concerns in the selected evidence.".to_owned(),
                    evidence: vec![evidence],
                    consent: ConsentRecord {
                        acknowledged: true,
                        local_only: true,
                        disclosure: "The selected local evidence will be reviewed by an agent."
                            .into(),
                    },
                    provenance: provenance(ProvenanceKind::AgentReview),
                };
                let context = AgentReviewContext::new(vec![CaptureContext {
                    capture_id: capture.capture_id.clone(),
                    dimensions: capture.dimensions.clone(),
                    content_sha256: capture.image.sha256.clone(),
                }])
                .map_err(|_| ReviewServiceError::InvalidEvidence)?;
                let token = CancellationToken::from_flag(lease.cancellation_flag());
                match ReviewCoordinator::new(Some(Arc::clone(adapter))).review(
                    &agent_request,
                    &context,
                    &token,
                ) {
                    Ok(response) => {
                        agent_status = response.status;
                        if response.status == OperationStatus::Success {
                            agent_findings = response.findings;
                            // Keep the adapter's validated name/version
                            // provenance with the persisted agent result. The
                            // findings remain in their separate manifest
                            // array and are never relabelled deterministic.
                            updated.provenance = response.provenance.clone();
                            updated.agent_findings.extend(agent_findings.clone());
                        }
                    }
                    Err(_) => agent_status = OperationStatus::Error,
                }
            } else {
                agent_status = OperationStatus::Unavailable;
            }
        }
        if updated.deterministic_findings.len() > MAX_FINDINGS {
            return Err(ReviewServiceError::Storage(StoreError::Quota(
                "finding count limit reached".into(),
            )));
        }
        let bytes = to_canonical_json(&updated).map_err(|_| ReviewServiceError::InvalidEvidence)?;
        self.store.replace_manifest(session_id, &bytes)?;
        Ok(ReviewResult {
            session_id: session_id.to_owned(),
            capture_id: selected_capture_id,
            deterministic_findings: deterministic,
            agent_status,
            agent_findings,
        })
    }

    fn compare(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let request = match typed_payload::<CompareRequest>(payload) {
            Ok(request) => request,
            Err(response) => return response_for_error(request_id, response),
        };
        let session_id = request
            .session_id
            .unwrap_or_else(|| DEFAULT_SESSION_ID.to_owned());
        let manifest = match self.read_manifest(&session_id) {
            Ok(manifest) => manifest,
            Err(error) => return review_error_response(&request_id, error),
        };
        let before = match self.review_input(&session_id, &manifest, &request.before_capture_id) {
            Ok(input) => input,
            Err(error) => return review_error_response(&request_id, error),
        };
        let after = match self.review_input(&session_id, &manifest, &request.after_capture_id) {
            Ok(input) => input,
            Err(error) => return review_error_response(&request_id, error),
        };
        match surfacecheck_review::compare(&before, &after) {
            Ok(comparison) => success(
                request_id,
                serde_json::to_value(comparison).unwrap_or_else(|_| serde_json::json!({})),
            ),
            Err(error) => error_response(
                &request_id,
                OperationStatus::Invalid,
                ErrorCode::InvalidEvidence,
                &error.to_string(),
            ),
        }
    }

    fn select_before_after(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let lease = match self.flight.begin(&request_id) {
            Ok(lease) => lease,
            Err(_) => {
                return error_response(
                    &request_id,
                    OperationStatus::Busy,
                    ErrorCode::OperationBusy,
                    "another operation is active",
                )
            }
        };
        let response = self.select_before_after_inner(&request_id, payload);
        self.flight.finish(&lease);
        response
    }

    fn select_before_after_inner(
        &self,
        request_id: &str,
        payload: serde_json::Value,
    ) -> RawResponse {
        let request = match typed_payload::<SelectBeforeAfterRequest>(payload) {
            Ok(request) => request,
            Err(response) => return response_for_error(request_id.to_owned(), response),
        };
        let manifest = match self.read_manifest(&request.session_id) {
            Ok(manifest) => manifest,
            Err(error) => return review_error_response(request_id, error),
        };
        let before =
            match self.review_input(&request.session_id, &manifest, &request.before_capture_id) {
                Ok(input) => input,
                Err(error) => return review_error_response(request_id, error),
            };
        let after =
            match self.review_input(&request.session_id, &manifest, &request.after_capture_id) {
                Ok(input) => input,
                Err(error) => return review_error_response(request_id, error),
            };
        let comparison = match surfacecheck_review::compare(&before, &after) {
            Ok(comparison) => comparison,
            Err(error) => {
                return error_response(
                    request_id,
                    OperationStatus::Invalid,
                    ErrorCode::InvalidEvidence,
                    &error.to_string(),
                )
            }
        };
        let mut manifest = manifest;
        manifest.comparison = Some(comparison.clone());
        manifest.before_after = Some(BeforeAfter {
            before_capture_id: request.before_capture_id,
            after_capture_id: request.after_capture_id,
            comparison_id: comparison.comparison_id.clone(),
        });
        let bytes = match to_canonical_json(&manifest) {
            Ok(bytes) => bytes,
            Err(_) => {
                return error_response(
                    request_id,
                    OperationStatus::Invalid,
                    ErrorCode::InvalidEvidence,
                    "manifest is invalid",
                )
            }
        };
        match self.store.replace_manifest(&request.session_id, &bytes) {
            Ok(()) => success(
                request_id.to_owned(),
                serde_json::to_value(comparison).unwrap_or_else(|_| serde_json::json!({})),
            ),
            Err(error) => store_response(request_id, error),
        }
    }

    fn annotate(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let lease = match self.flight.begin(&request_id) {
            Ok(lease) => lease,
            Err(_) => {
                return error_response(
                    &request_id,
                    OperationStatus::Busy,
                    ErrorCode::OperationBusy,
                    "another operation is active",
                )
            }
        };
        let response = self.annotate_inner(&request_id, payload);
        self.flight.finish(&lease);
        response
    }

    fn annotate_inner(&self, request_id: &str, payload: serde_json::Value) -> RawResponse {
        let request = match typed_payload::<surfacecheck_core::AnnotateRequest>(payload) {
            Ok(request) => request,
            Err(response) => return response_for_error(request_id.to_owned(), response),
        };
        let mut manifest = match self.read_manifest(&request.session_id) {
            Ok(manifest) => manifest,
            Err(error) => return review_error_response(request_id, error),
        };
        manifest.user_note = Some(request.note);
        match to_canonical_json(&manifest)
            .map_err(|_| StoreError::Manifest("manifest is invalid".into()))
            .and_then(|bytes| self.store.replace_manifest(&request.session_id, &bytes))
        {
            Ok(()) => success(
                request_id.to_owned(),
                serde_json::json!({"annotated": true}),
            ),
            Err(error) => store_response(request_id, error),
        }
    }

    fn export(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let lease = match self.flight.begin(&request_id) {
            Ok(lease) => lease,
            Err(_) => {
                return error_response(
                    &request_id,
                    OperationStatus::Busy,
                    ErrorCode::OperationBusy,
                    "another operation is active",
                )
            }
        };
        let response = self.export_inner(&request_id, payload);
        self.flight.finish(&lease);
        response
    }

    fn export_inner(&self, request_id: &str, payload: serde_json::Value) -> RawResponse {
        let request = match typed_payload::<surfacecheck_core::ExportRequest>(payload) {
            Ok(request) => request,
            Err(response) => return response_for_error(request_id.to_owned(), response),
        };
        match self.store.export_to_file(&request.session_id) {
            Ok(artifact) => success(
                request_id.to_owned(),
                serde_json::to_value(ExportResult {
                    session_id: request.session_id,
                    relative_path: artifact.relative_path,
                    format: "ustar",
                    bytes: artifact.bytes,
                    sha256: artifact.sha256,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            ),
            Err(error) => store_response(request_id, error),
        }
    }

    fn handoff(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let request = match self.resolve_handoff(payload) {
            Ok(request) => request,
            Err(error) => return review_error_response(&request_id, error),
        };
        let lease = match self.flight.begin(&request_id) {
            Ok(lease) => lease,
            Err(_) => {
                return error_response(
                    &request_id,
                    OperationStatus::Busy,
                    ErrorCode::OperationBusy,
                    "another operation is active",
                )
            }
        };
        let token = CancellationToken::from_flag(lease.cancellation_flag());
        let response =
            match PremonitionCoordinator::new(self.premonition.clone()).handoff(&request, &token) {
                Ok(response) => success(
                    request_id.clone(),
                    serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({})),
                ),
                Err(error) => premonition_error_response(&request_id, error),
            };
        self.flight.finish(&lease);
        response
    }

    fn resolve_handoff(
        &self,
        payload: serde_json::Value,
    ) -> Result<PremonitionHandoffRequest, ReviewServiceError> {
        if let Ok(request) = serde_json::from_value::<PremonitionHandoffRequest>(payload.clone()) {
            request
                .validate()
                .map_err(|_| ReviewServiceError::InvalidEvidence)?;
            return Ok(request);
        }
        let selector: HandoffFindingRequest =
            serde_json::from_value(payload).map_err(|_| ReviewServiceError::InvalidEvidence)?;
        selector
            .validate()
            .map_err(|_| ReviewServiceError::InvalidEvidence)?;
        let manifest = self.read_manifest(&selector.session_id)?;
        let finding = manifest
            .agent_findings
            .iter()
            .find(|finding| finding.finding_id == selector.finding_id)
            .ok_or(ReviewServiceError::NotFound)?;
        let first_evidence = finding
            .evidence
            .first()
            .ok_or(ReviewServiceError::InvalidEvidence)?;
        // Re-read every cited object immediately before constructing the
        // external envelope. This closes the gap between manifest validation
        // and handoff if a local evidence file was replaced or truncated.
        for evidence in &finding.evidence {
            self.store
                .read_capture(&selector.session_id, &evidence.capture_id)?;
        }
        let capture = first_evidence.capture_id.clone();
        Ok(PremonitionHandoffRequest {
            schema_version: SCHEMA_VERSION,
            handoff_id: format!("handoff-{}", selector.finding_id),
            adapter_protocol_version: PREMONITION_PROTOCOL_VERSION,
            defect: DefectEnvelope {
                defect_id: format!("defect-{}", selector.finding_id),
                finding_id: finding.finding_id.clone(),
                finding_source: FindingSource::Agent,
                capture_id: capture,
                category: finding.category,
                severity: finding.severity,
                explanation: finding.explanation.clone(),
                evidence: finding.evidence.clone(),
                suggested_next_action: finding.suggested_next_action.clone(),
                provenance: provenance(ProvenanceKind::Handoff),
            },
            consent: selector.consent,
        })
    }

    fn cancel(&self, request_id: String, payload: serde_json::Value) -> RawResponse {
        let request = match typed_payload::<surfacecheck_core::CancelRequest>(payload) {
            Ok(request) => request,
            Err(response) => return response_for_error(request_id, response),
        };
        let cancelled = self
            .flight
            .cancel(&request.operation_id, request.generation);
        success(
            request_id,
            serde_json::to_value(CancelResult {
                operation_id: request.operation_id,
                generation: request.generation,
                cancelled,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
    }

    fn read_manifest(&self, session_id: &str) -> Result<EvidenceManifest, ReviewServiceError> {
        let bytes = self
            .store
            .read_manifest(session_id)
            .map_err(|error| match error {
                StoreError::InvalidPath(_) => ReviewServiceError::NotFound,
                StoreError::Io(error) if error.kind() == io::ErrorKind::NotFound => {
                    ReviewServiceError::NotFound
                }
                error => ReviewServiceError::Storage(error),
            })?;
        from_json(&bytes).map_err(|_| ReviewServiceError::InvalidEvidence)
    }

    fn review_input(
        &self,
        session_id: &str,
        manifest: &EvidenceManifest,
        capture_id: &str,
    ) -> Result<ReviewInput, ReviewServiceError> {
        let capture = manifest
            .captures
            .iter()
            .find(|capture| capture.capture_id == capture_id)
            .ok_or(ReviewServiceError::NotFound)?;
        let png = self.store.read_capture(session_id, capture_id)?;
        let image =
            surfacecheck_capture::decode_png(&png, surfacecheck_capture::PngLimits::default())
                .map_err(|_| ReviewServiceError::InvalidEvidence)?;
        Ok(ReviewInput {
            capture_id: capture.capture_id.clone(),
            image,
            scale: capture.scale.clone(),
            stale: capture.stale,
            duplicate_of: None,
            expected_dimensions: Some(capture.dimensions.clone()),
        })
    }
}

pub fn run_foreground(service: RuntimeService) -> Result<(), ServiceError> {
    let paths = RuntimePaths::from_environment()?;
    #[cfg(unix)]
    {
        use std::time::Duration;

        let listener = paths.bind()?;
        let uid = unsafe { libc::geteuid() };
        let service = Arc::new(service);
        let clients = Arc::new(AtomicUsize::new(0));
        loop {
            let (mut stream, _) = listener.accept()?;
            let current = clients.fetch_add(1, Ordering::AcqRel) + 1;
            if current > MAX_CLIENTS {
                clients.fetch_sub(1, Ordering::AcqRel);
                drop(stream);
                continue;
            }
            let service = Arc::clone(&service);
            let worker_clients = Arc::clone(&clients);
            let result_clients = Arc::clone(&clients);
            let result = std::thread::Builder::new()
                .name("surfacecheck-client".into())
                .spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(35)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(35)));
                    if authenticate_peer(&stream, uid).is_ok() {
                        if let Ok(frame) = read_frame(&mut stream) {
                            let response = service.handle_frame(&frame);
                            let _ = write_frame(&mut stream, &response);
                        }
                    }
                    worker_clients.fetch_sub(1, Ordering::AcqRel);
                });
            if let Err(error) = result {
                result_clients.fetch_sub(1, Ordering::AcqRel);
                return Err(ServiceError::Io(error));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = service;
        Err(ServiceError::UnsupportedPlatform)
    }
}

#[allow(clippy::result_large_err)]
fn typed_payload<T: DeserializeOwned + Validate>(
    payload: serde_json::Value,
) -> Result<T, RawResponse> {
    let value: T = serde_json::from_value(payload).map_err(|_| {
        error_response(
            "invalid-request",
            OperationStatus::Invalid,
            ErrorCode::InvalidRequest,
            "payload is malformed",
        )
    })?;
    value.validate().map_err(|_| {
        error_response(
            "invalid-request",
            OperationStatus::Invalid,
            ErrorCode::InvalidRequest,
            "payload is invalid",
        )
    })?;
    Ok(value)
}

#[derive(Debug)]
enum ReviewServiceError {
    NotFound,
    InvalidEvidence,
    Storage(StoreError),
}

impl From<StoreError> for ReviewServiceError {
    fn from(error: StoreError) -> Self {
        Self::Storage(error)
    }
}

fn empty_manifest(session_id: &str) -> EvidenceManifest {
    EvidenceManifest {
        schema_version: SCHEMA_VERSION,
        session_id: session_id.to_owned(),
        created_at: now_seconds(),
        captures: Vec::new(),
        user_note: None,
        deterministic_findings: Vec::new(),
        agent_findings: Vec::new(),
        comparison: None,
        before_after: None,
        provenance: provenance(ProvenanceKind::LocalCapture),
    }
}

fn provenance(kind: ProvenanceKind) -> Provenance {
    Provenance {
        kind,
        producer: "surfacecheckd".into(),
        producer_version: PRODUCER_VERSION.into(),
        producer_commit: producer_commit(),
        tool_versions: Vec::new(),
    }
}

fn now_seconds() -> u64 {
    std::env::var("SURFACECHECK_CLOCK")
        .ok()
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs())
        })
        .unwrap_or(0)
}

fn producer_commit() -> String {
    let candidate = std::env::var("SURFACECHECK_COMMIT").unwrap_or_default();
    if (7..=64).contains(&candidate.len()) && candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        candidate
    } else {
        "unknown".into()
    }
}

/// Read compositor scale supplied by the capture integration without retaining
/// monitor names, titles, or paths.  Omarchy's capture adapter can set this to
/// `x,y` (or a single value for both axes); malformed values fail closed to the
/// neutral scale rather than making a capture unverifiable.
fn capture_scale() -> Scale {
    let parsed = std::env::var("SURFACECHECK_SCALE").ok().and_then(|value| {
        let fields: Vec<_> = value.split(',').collect();
        if fields.len() > 2 {
            return None;
        }
        let x = fields.first()?.trim().parse::<f64>().ok()?;
        let y = fields
            .get(1)
            .map_or(Some(x), |field| field.trim().parse::<f64>().ok())?;
        let scale = Scale { x, y };
        scale.validate().ok().map(|_| scale)
    });
    parsed.unwrap_or(Scale { x: 1.0, y: 1.0 })
}

/// Keep only a deliberately small, user-supplied application label.  The
/// Hyprland address, title, class, URL, and complete path never enter the
/// manifest, even when a caller accidentally supplies one as an alias.
fn redact_alias(alias: &str) -> Option<String> {
    let mut redacted = String::new();
    for character in alias.chars().take(128) {
        let safe = character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ');
        redacted.push(if safe { character } else { '_' });
    }
    let redacted = redacted.trim().to_owned();
    (!redacted.is_empty()).then_some(redacted)
}

fn encode(response: &RawResponse) -> Vec<u8> {
    serde_json::to_vec(response)
        .unwrap_or_else(|_| b"{\"schemaVersion\":1,\"requestId\":\"error\",\"status\":\"error\",\"result\":null,\"error\":{\"code\":\"internal\",\"message\":\"response encoding failed\",\"retryable\":false}}".to_vec())
}

fn success(request_id: String, result: serde_json::Value) -> RawResponse {
    RawResponse {
        schema_version: SCHEMA_VERSION,
        request_id,
        status: OperationStatus::Success,
        result: Some(result),
        error: None,
    }
}

fn error_response(
    request_id: &str,
    status: OperationStatus,
    code: ErrorCode,
    message: &str,
) -> RawResponse {
    RawResponse {
        schema_version: SCHEMA_VERSION,
        request_id: request_id.to_owned(),
        status,
        result: None,
        error: Some(ErrorEnvelope {
            code,
            message: message.chars().take(4096).collect(),
            retryable: matches!(
                status,
                OperationStatus::Busy | OperationStatus::Timeout | OperationStatus::Unavailable
            ),
        }),
    }
}

fn response_for_error(request_id: String, response: RawResponse) -> RawResponse {
    if response.request_id == "invalid-request" {
        error_response(
            &request_id,
            response.status,
            response
                .error
                .map(|error| error.code)
                .unwrap_or(ErrorCode::InvalidRequest),
            "request payload is invalid",
        )
    } else {
        response
    }
}

fn store_response(request_id: &str, error: StoreError) -> RawResponse {
    let (status, code) = match error {
        StoreError::Quota(_) => (OperationStatus::Error, ErrorCode::StorageLimit),
        StoreError::InvalidPath(_) | StoreError::Manifest(_) => {
            (OperationStatus::Invalid, ErrorCode::InvalidEvidence)
        }
        _ => (OperationStatus::Error, ErrorCode::Internal),
    };
    error_response(
        request_id,
        status,
        code,
        "evidence storage operation failed",
    )
}

fn review_error_response(request_id: &str, error: ReviewServiceError) -> RawResponse {
    let (status, code, message) = match error {
        ReviewServiceError::NotFound => (
            OperationStatus::Invalid,
            ErrorCode::NotFound,
            "review evidence was not found",
        ),
        ReviewServiceError::InvalidEvidence => (
            OperationStatus::Invalid,
            ErrorCode::InvalidEvidence,
            "review evidence is invalid",
        ),
        ReviewServiceError::Storage(StoreError::Quota(_)) => (
            OperationStatus::Error,
            ErrorCode::StorageLimit,
            "evidence storage limit was reached",
        ),
        ReviewServiceError::Storage(_) => (
            OperationStatus::Error,
            ErrorCode::Internal,
            "evidence storage operation failed",
        ),
    };
    error_response(request_id, status, code, message)
}

fn premonition_error_response(request_id: &str, error: PremonitionHandoffError) -> RawResponse {
    let (status, code, message) = match error {
        PremonitionHandoffError::InvalidRequest
        | PremonitionHandoffError::ExternalConsentRequired => (
            OperationStatus::Invalid,
            ErrorCode::InvalidRequest,
            "explicit external handoff consent and a valid defect are required",
        ),
        PremonitionHandoffError::AdapterUnavailable => (
            OperationStatus::Unavailable,
            ErrorCode::AdapterUnavailable,
            "Premonition adapter is unavailable",
        ),
        PremonitionHandoffError::AdapterIncompatible => (
            OperationStatus::Unavailable,
            ErrorCode::AdapterIncompatible,
            "Premonition adapter protocol is incompatible",
        ),
        PremonitionHandoffError::Cancelled => (
            OperationStatus::Cancelled,
            ErrorCode::SelectionCancelled,
            "Premonition handoff was cancelled",
        ),
        PremonitionHandoffError::Timeout => (
            OperationStatus::Timeout,
            ErrorCode::OperationTimeout,
            "Premonition handoff timed out",
        ),
        PremonitionHandoffError::AdapterCrashed => (
            OperationStatus::Error,
            ErrorCode::ToolFailed,
            "Premonition adapter failed",
        ),
        PremonitionHandoffError::MalformedOutput => (
            OperationStatus::Error,
            ErrorCode::MalformedAgentOutput,
            "Premonition response was malformed",
        ),
        PremonitionHandoffError::OutputTooLarge => (
            OperationStatus::Error,
            ErrorCode::StorageLimit,
            "Premonition response exceeded the bounded limit",
        ),
    };
    error_response(request_id, status, code, message)
}

fn capture_message(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::MissingTool => "required capture tool is not installed",
        OperationStatus::Cancelled => "capture was cancelled",
        OperationStatus::Timeout => "capture tool timed out",
        OperationStatus::Invalid => "capture evidence is invalid",
        _ => "capture failed",
    }
}

fn store_error(error: StoreError) -> ServiceError {
    ServiceError::Io(io::Error::other(error.to_string()))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile_like::TempDir;

    // A tiny local temp helper avoids adding a third-party dependency.
    mod tempfile_like {
        use std::path::PathBuf;
        pub struct TempDir(PathBuf);
        impl TempDir {
            pub fn new(label: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "surfacecheck-runtime-{label}-{}",
                    std::process::id()
                ));
                let _ = std::fs::remove_dir_all(&path);
                Self(path)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn malformed_request_gets_one_versioned_response() {
        let root = TempDir::new("malformed");
        let service = RuntimeService::new(StoreConfig::new(root.path())).expect("service");
        let response: serde_json::Value =
            serde_json::from_slice(&service.handle_frame(b"{")).expect("response json");
        assert_eq!(response["schemaVersion"], 1);
        assert_eq!(response["status"], "invalid");
    }

    #[test]
    fn hostile_request_id_is_not_reflected_into_the_response() {
        let root = TempDir::new("hostile-id");
        let service = RuntimeService::new(StoreConfig::new(root.path())).expect("service");
        let request = serde_json::json!({
            "schemaVersion": 1,
            "requestId": "../../secret",
            "command": "status",
            "payload": {}
        });
        let response: serde_json::Value =
            serde_json::from_slice(&service.handle_frame(&serde_json::to_vec(&request).unwrap()))
                .expect("response");
        assert_eq!(response["requestId"], "invalid-request");
        assert_eq!(response["status"], "invalid");
    }

    #[test]
    fn service_status_reports_agent_and_handoff_as_unavailable_without_adapters() {
        let root = TempDir::new("status");
        let service = RuntimeService::new(StoreConfig::new(root.path())).expect("service");
        let request = serde_json::json!({
            "schemaVersion": 1,
            "requestId": "request-1",
            "command": "status",
            "payload": {}
        });
        let response: serde_json::Value =
            serde_json::from_slice(&service.handle_frame(&serde_json::to_vec(&request).unwrap()))
                .expect("response");
        assert_eq!(response["status"], "success");
        assert_eq!(response["result"]["agent"], "unavailable");
        assert_eq!(response["result"]["handoff"], "unavailable");
    }

    #[test]
    fn status_rejects_unknown_payload_fields() {
        let root = TempDir::new("status-payload");
        let service = RuntimeService::new(StoreConfig::new(root.path())).expect("service");
        let request = serde_json::json!({
            "schemaVersion": 1,
            "requestId": "request-3",
            "command": "status",
            "payload": {"unexpected": true}
        });
        let response: serde_json::Value =
            serde_json::from_slice(&service.handle_frame(&serde_json::to_vec(&request).unwrap()))
                .expect("response");
        assert_eq!(response["status"], "invalid");
        assert_eq!(response["error"]["code"], "invalid_request");
    }

    #[test]
    fn cancel_without_matching_operation_is_safe() {
        let root = TempDir::new("cancel");
        let service = RuntimeService::new(StoreConfig::new(root.path())).expect("service");
        let request = serde_json::json!({
            "schemaVersion": 1,
            "requestId": "request-2",
            "command": "cancel",
            "payload": {"operationId":"capture-1","generation":1}
        });
        let response: serde_json::Value =
            serde_json::from_slice(&service.handle_frame(&serde_json::to_vec(&request).unwrap()))
                .expect("response");
        assert_eq!(response["result"]["cancelled"], false);
    }
}
