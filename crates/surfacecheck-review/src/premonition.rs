//! Version-negotiated, mockable Premonition handoff.
//!
//! No production Premonition transport is assumed.  Callers inject an
//! adapter only when a stable protocol is available; otherwise the
//! coordinator reports an honest unavailable or incompatible state.

use crate::agent::CancellationToken;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use surfacecheck_core::{
    from_json, to_canonical_json, PremonitionHandoffRequest, PremonitionHandoffResponse, Validate,
    MAX_JSON_FRAME_BYTES, PREMONITION_PROTOCOL_VERSION,
};

pub trait PremonitionAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn protocol_version(&self) -> u16;
    fn handoff(
        &self,
        request: &PremonitionHandoffRequest,
        cancellation: &CancellationToken,
    ) -> Result<PremonitionHandoffResponse, PremonitionAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremonitionAdapterError {
    Unavailable,
    Timeout,
    Crashed,
    MalformedOutput,
}

impl fmt::Display for PremonitionAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Premonition adapter is unavailable",
            Self::Timeout => "Premonition handoff timed out",
            Self::Crashed => "Premonition adapter failed",
            Self::MalformedOutput => "Premonition adapter output is malformed",
        })
    }
}

impl Error for PremonitionAdapterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremonitionHandoffError {
    InvalidRequest,
    ExternalConsentRequired,
    AdapterUnavailable,
    AdapterIncompatible,
    Cancelled,
    Timeout,
    AdapterCrashed,
    MalformedOutput,
    OutputTooLarge,
}

impl fmt::Display for PremonitionHandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "Premonition handoff request is invalid",
            Self::ExternalConsentRequired => "explicit external handoff consent is required",
            Self::AdapterUnavailable => "Premonition adapter is unavailable",
            Self::AdapterIncompatible => "Premonition adapter protocol is incompatible",
            Self::Cancelled => "Premonition handoff was cancelled",
            Self::Timeout => "Premonition handoff timed out",
            Self::AdapterCrashed => "Premonition adapter failed",
            Self::MalformedOutput => "Premonition output does not conform to the v1 contract",
            Self::OutputTooLarge => "Premonition output exceeds the bounded response limit",
        })
    }
}

impl Error for PremonitionHandoffError {}

pub struct PremonitionCoordinator {
    adapter: Option<Arc<dyn PremonitionAdapter>>,
}

impl PremonitionCoordinator {
    pub fn new(adapter: Option<Arc<dyn PremonitionAdapter>>) -> Self {
        Self { adapter }
    }

    pub fn handoff(
        &self,
        request: &PremonitionHandoffRequest,
        cancellation: &CancellationToken,
    ) -> Result<PremonitionHandoffResponse, PremonitionHandoffError> {
        if !request.consent.acknowledged || request.consent.local_only {
            return Err(PremonitionHandoffError::ExternalConsentRequired);
        }
        request
            .validate()
            .map_err(|_| PremonitionHandoffError::InvalidRequest)?;
        if cancellation.is_cancelled() {
            return Err(PremonitionHandoffError::Cancelled);
        }
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(PremonitionHandoffError::AdapterUnavailable)?;
        if request.adapter_protocol_version != PREMONITION_PROTOCOL_VERSION
            || adapter.protocol_version() != request.adapter_protocol_version
        {
            return Err(PremonitionHandoffError::AdapterIncompatible);
        }
        let response = adapter
            .handoff(request, cancellation)
            .map_err(|error| match error {
                PremonitionAdapterError::Unavailable => PremonitionHandoffError::AdapterUnavailable,
                PremonitionAdapterError::Timeout => PremonitionHandoffError::Timeout,
                PremonitionAdapterError::Crashed => PremonitionHandoffError::AdapterCrashed,
                PremonitionAdapterError::MalformedOutput => {
                    PremonitionHandoffError::MalformedOutput
                }
            })?;
        if cancellation.is_cancelled() {
            return Err(PremonitionHandoffError::Cancelled);
        }
        response
            .validate()
            .map_err(|_| PremonitionHandoffError::MalformedOutput)?;
        if response.handoff_id != request.handoff_id {
            return Err(PremonitionHandoffError::MalformedOutput);
        }
        let encoded =
            to_canonical_json(&response).map_err(|_| PremonitionHandoffError::MalformedOutput)?;
        if encoded.len() > MAX_JSON_FRAME_BYTES {
            return Err(PremonitionHandoffError::OutputTooLarge);
        }
        Ok(response)
    }
}

#[derive(Debug, Clone)]
pub enum MockPremonitionBehavior {
    Response(PremonitionHandoffResponse),
    Raw(Vec<u8>),
    Timeout,
    Crash,
    Unavailable,
}

pub struct MockPremonitionAdapter {
    protocol_version: u16,
    behavior: Mutex<MockPremonitionBehavior>,
}

impl MockPremonitionAdapter {
    pub fn new(behavior: MockPremonitionBehavior) -> Self {
        Self {
            protocol_version: PREMONITION_PROTOCOL_VERSION,
            behavior: Mutex::new(behavior),
        }
    }

    pub fn with_protocol_version(mut self, protocol_version: u16) -> Self {
        self.protocol_version = protocol_version;
        self
    }
}

impl PremonitionAdapter for MockPremonitionAdapter {
    fn name(&self) -> &str {
        "mock-premonition"
    }

    fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    fn handoff(
        &self,
        _request: &PremonitionHandoffRequest,
        cancellation: &CancellationToken,
    ) -> Result<PremonitionHandoffResponse, PremonitionAdapterError> {
        if cancellation.is_cancelled() {
            return Err(PremonitionAdapterError::Timeout);
        }
        let behavior = self
            .behavior
            .lock()
            .map_err(|_| PremonitionAdapterError::Crashed)?
            .clone();
        match behavior {
            MockPremonitionBehavior::Response(response) => Ok(response),
            MockPremonitionBehavior::Raw(raw) => {
                from_json(&raw).map_err(|_| PremonitionAdapterError::MalformedOutput)
            }
            MockPremonitionBehavior::Timeout => Err(PremonitionAdapterError::Timeout),
            MockPremonitionBehavior::Crash => Err(PremonitionAdapterError::Crashed),
            MockPremonitionBehavior::Unavailable => Err(PremonitionAdapterError::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use surfacecheck_core::{
        AgentCategory, ConsentRecord, DefectEnvelope, FindingSource, OperationStatus, Provenance,
        ProvenanceKind, Severity, SCHEMA_VERSION,
    };

    fn provenance() -> Provenance {
        Provenance {
            kind: ProvenanceKind::Handoff,
            producer: "test".into(),
            producer_version: "0.1".into(),
            producer_commit: "fixture".into(),
            tool_versions: Vec::new(),
        }
    }

    fn request() -> PremonitionHandoffRequest {
        PremonitionHandoffRequest {
            schema_version: SCHEMA_VERSION,
            handoff_id: "handoff-1".into(),
            adapter_protocol_version: PREMONITION_PROTOCOL_VERSION,
            defect: DefectEnvelope {
                defect_id: "defect-1".into(),
                finding_id: "finding-1".into(),
                finding_source: FindingSource::Agent,
                capture_id: "capture-1".into(),
                category: AgentCategory::Layout,
                severity: Severity::Medium,
                explanation: "A visible defect.".into(),
                evidence: vec![surfacecheck_core::EvidenceRef {
                    capture_id: "capture-1".into(),
                    content_sha256: "0".repeat(64),
                    region: surfacecheck_core::EvidenceRegion {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                }],
                suggested_next_action: "Inspect the spacing.".into(),
                provenance: provenance(),
            },
            consent: ConsentRecord {
                acknowledged: true,
                local_only: false,
                disclosure: "The selected defect will be handed to Premonition.".into(),
            },
        }
    }

    fn response() -> PremonitionHandoffResponse {
        PremonitionHandoffResponse {
            schema_version: SCHEMA_VERSION,
            handoff_id: "handoff-1".into(),
            status: OperationStatus::Success,
            external_reference: Some("premonition-1".into()),
            error: None,
        }
    }

    #[test]
    fn versioned_handoff_is_explicit_and_mockable() {
        let adapter = Arc::new(MockPremonitionAdapter::new(
            MockPremonitionBehavior::Response(response()),
        ));
        let output = PremonitionCoordinator::new(Some(adapter))
            .handoff(&request(), &CancellationToken::new())
            .expect("handoff");
        assert_eq!(output.external_reference.as_deref(), Some("premonition-1"));
    }

    #[test]
    fn unavailable_incompatible_and_malformed_are_honest() {
        assert_eq!(
            PremonitionCoordinator::new(None).handoff(&request(), &CancellationToken::new()),
            Err(PremonitionHandoffError::AdapterUnavailable)
        );
        let adapter = Arc::new(
            MockPremonitionAdapter::new(MockPremonitionBehavior::Response(response()))
                .with_protocol_version(2),
        );
        assert_eq!(
            PremonitionCoordinator::new(Some(adapter))
                .handoff(&request(), &CancellationToken::new()),
            Err(PremonitionHandoffError::AdapterIncompatible)
        );
        let adapter = Arc::new(MockPremonitionAdapter::new(MockPremonitionBehavior::Raw(
            br#"{"schemaVersion":1,"handoffId":"handoff-1","status":"success","externalReference":"x","error":null,"extra":true}"#.to_vec(),
        )));
        assert_eq!(
            PremonitionCoordinator::new(Some(adapter))
                .handoff(&request(), &CancellationToken::new()),
            Err(PremonitionHandoffError::MalformedOutput)
        );
    }

    #[test]
    fn local_only_or_cancelled_requests_never_leave_the_process() {
        let mut local = request();
        local.consent.local_only = true;
        let adapter = Arc::new(MockPremonitionAdapter::new(
            MockPremonitionBehavior::Response(response()),
        ));
        assert_eq!(
            PremonitionCoordinator::new(Some(adapter)).handoff(&local, &CancellationToken::new()),
            Err(PremonitionHandoffError::ExternalConsentRequired)
        );
        let token = CancellationToken::new();
        token.cancel();
        let adapter = Arc::new(MockPremonitionAdapter::new(
            MockPremonitionBehavior::Response(response()),
        ));
        assert_eq!(
            PremonitionCoordinator::new(Some(adapter)).handoff(&request(), &token),
            Err(PremonitionHandoffError::Cancelled)
        );
    }
}
