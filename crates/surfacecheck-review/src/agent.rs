//! Explicit, local-only agent review orchestration.
//!
//! The production implementation deliberately has no network client.  An
//! adapter is injected by the caller, which keeps consent, bounds, and output
//! validation in this crate while making a future local agent process
//! mockable in tests.

use std::error::Error;
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use surfacecheck_core::{
    from_json, to_canonical_json, AgentReviewRequest, AgentReviewResponse, Dimensions, EvidenceRef,
    OperationStatus, Validate, MAX_AGENT_RESPONSE_BYTES, MAX_CAPTURE_COUNT, MAX_EVIDENCE_REFS,
    MAX_JSON_FRAME_BYTES,
};

pub const AGENT_ADAPTER_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a review operation to the service-owned cancellation flag.
    /// Keeping the flag behind the small token type prevents adapters from
    /// mutating service state while still allowing cancellation to interrupt
    /// an in-flight review.
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureContext {
    pub capture_id: String,
    pub dimensions: Dimensions,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentReviewContext {
    pub captures: Vec<CaptureContext>,
}

impl AgentReviewContext {
    pub fn new(captures: Vec<CaptureContext>) -> Result<Self, AgentReviewError> {
        if captures.is_empty() || captures.len() > MAX_CAPTURE_COUNT {
            return Err(AgentReviewError::InvalidContext);
        }
        let mut context = Self { captures };
        for capture in &context.captures {
            if !valid_id(&capture.capture_id)
                || !valid_sha256(&capture.content_sha256)
                || capture.dimensions.validate().is_err()
            {
                return Err(AgentReviewError::InvalidContext);
            }
        }
        for (index, capture) in context.captures.iter().enumerate() {
            if context.captures[..index]
                .iter()
                .any(|previous| previous.capture_id == capture.capture_id)
            {
                return Err(AgentReviewError::InvalidContext);
            }
        }
        context.captures.shrink_to_fit();
        Ok(context)
    }

    fn capture(&self, id: &str) -> Option<&CaptureContext> {
        self.captures
            .iter()
            .find(|capture| capture.capture_id == id)
    }
}

pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn protocol_version(&self) -> u16;
    fn review(
        &self,
        request: &AgentReviewRequest,
        cancellation: &CancellationToken,
    ) -> Result<AgentReviewResponse, AgentAdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAdapterError {
    Unavailable,
    Timeout,
    Crashed,
    MalformedOutput,
}

impl fmt::Display for AgentAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "agent adapter is unavailable",
            Self::Timeout => "agent review timed out",
            Self::Crashed => "agent adapter failed",
            Self::MalformedOutput => "agent adapter output is malformed",
        })
    }
}

impl Error for AgentAdapterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentReviewError {
    InvalidRequest,
    InvalidContext,
    ConsentRequired,
    AdapterUnavailable,
    AdapterIncompatible,
    Cancelled,
    Timeout,
    AdapterCrashed,
    MalformedOutput,
    OutputTooLarge,
    FindingOutsideEvidence,
    DuplicateFinding,
}

impl fmt::Display for AgentReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "agent review request is invalid",
            Self::InvalidContext => "agent review context is invalid",
            Self::ConsentRequired => "explicit local agent consent is required",
            Self::AdapterUnavailable => "agent adapter is unavailable",
            Self::AdapterIncompatible => "agent adapter protocol is incompatible",
            Self::Cancelled => "agent review was cancelled",
            Self::Timeout => "agent review timed out",
            Self::AdapterCrashed => "agent adapter failed",
            Self::MalformedOutput => "agent output does not conform to the v1 contract",
            Self::OutputTooLarge => "agent output exceeds the bounded response limit",
            Self::FindingOutsideEvidence => {
                "agent finding cites evidence outside the selected capture"
            }
            Self::DuplicateFinding => "agent output contains duplicate finding IDs",
        })
    }
}

impl Error for AgentReviewError {}

pub struct ReviewCoordinator {
    adapter: Option<Arc<dyn AgentAdapter>>,
}

impl ReviewCoordinator {
    pub fn new(adapter: Option<Arc<dyn AgentAdapter>>) -> Self {
        Self { adapter }
    }

    pub fn review(
        &self,
        request: &AgentReviewRequest,
        context: &AgentReviewContext,
        cancellation: &CancellationToken,
    ) -> Result<AgentReviewResponse, AgentReviewError> {
        if !request.consent.acknowledged || !request.consent.local_only {
            return Err(AgentReviewError::ConsentRequired);
        }
        request
            .validate()
            .map_err(|_| AgentReviewError::InvalidRequest)?;
        validate_selected_evidence(request, context)?;
        if cancellation.is_cancelled() {
            return Err(AgentReviewError::Cancelled);
        }
        let adapter = self
            .adapter
            .as_ref()
            .ok_or(AgentReviewError::AdapterUnavailable)?;
        if adapter.protocol_version() != AGENT_ADAPTER_PROTOCOL_VERSION {
            return Err(AgentReviewError::AdapterIncompatible);
        }
        let response = adapter
            .review(request, cancellation)
            .map_err(|error| match error {
                AgentAdapterError::Unavailable => AgentReviewError::AdapterUnavailable,
                AgentAdapterError::Timeout => AgentReviewError::Timeout,
                AgentAdapterError::Crashed => AgentReviewError::AdapterCrashed,
                AgentAdapterError::MalformedOutput => AgentReviewError::MalformedOutput,
            })?;
        if cancellation.is_cancelled() {
            return Err(AgentReviewError::Cancelled);
        }
        validate_agent_response(&response, request, context)
    }
}

fn validate_selected_evidence(
    request: &AgentReviewRequest,
    context: &AgentReviewContext,
) -> Result<(), AgentReviewError> {
    if request.evidence.len() > MAX_EVIDENCE_REFS {
        return Err(AgentReviewError::InvalidRequest);
    }
    for evidence in &request.evidence {
        validate_evidence(evidence, context)?;
    }
    if !request
        .evidence
        .iter()
        .any(|evidence| evidence.capture_id == request.capture_id)
    {
        return Err(AgentReviewError::InvalidRequest);
    }
    Ok(())
}

fn validate_evidence(
    evidence: &EvidenceRef,
    context: &AgentReviewContext,
) -> Result<(), AgentReviewError> {
    let capture = context
        .capture(&evidence.capture_id)
        .ok_or(AgentReviewError::FindingOutsideEvidence)?;
    if capture.content_sha256 != evidence.content_sha256
        || u64::from(evidence.region.x) + u64::from(evidence.region.width)
            > u64::from(capture.dimensions.width)
        || u64::from(evidence.region.y) + u64::from(evidence.region.height)
            > u64::from(capture.dimensions.height)
    {
        return Err(AgentReviewError::FindingOutsideEvidence);
    }
    Ok(())
}

fn validate_agent_response(
    response: &AgentReviewResponse,
    request: &AgentReviewRequest,
    context: &AgentReviewContext,
) -> Result<AgentReviewResponse, AgentReviewError> {
    response
        .validate()
        .map_err(|_| AgentReviewError::MalformedOutput)?;
    if response.review_id != request.review_id {
        return Err(AgentReviewError::MalformedOutput);
    }
    let encoded = to_canonical_json(response).map_err(|_| AgentReviewError::MalformedOutput)?;
    if encoded.len() > MAX_AGENT_RESPONSE_BYTES || encoded.len() > MAX_JSON_FRAME_BYTES {
        return Err(AgentReviewError::OutputTooLarge);
    }
    if response.status != OperationStatus::Success {
        return Ok(response.clone());
    }
    let mut finding_ids = std::collections::HashSet::new();
    for finding in &response.findings {
        if !finding_ids.insert(finding.finding_id.as_str()) {
            return Err(AgentReviewError::DuplicateFinding);
        }
        for evidence in &finding.evidence {
            validate_evidence(evidence, context)?;
        }
    }
    Ok(response.clone())
}

#[derive(Debug, Clone)]
pub enum MockAgentBehavior {
    Response(AgentReviewResponse),
    Raw(Vec<u8>),
    Timeout,
    Crash,
    Unavailable,
}

pub struct MockAgentAdapter {
    protocol_version: u16,
    behavior: Mutex<MockAgentBehavior>,
}

impl MockAgentAdapter {
    pub fn new(behavior: MockAgentBehavior) -> Self {
        Self {
            protocol_version: AGENT_ADAPTER_PROTOCOL_VERSION,
            behavior: Mutex::new(behavior),
        }
    }

    pub fn with_protocol_version(mut self, protocol_version: u16) -> Self {
        self.protocol_version = protocol_version;
        self
    }
}

impl AgentAdapter for MockAgentAdapter {
    fn name(&self) -> &str {
        "mock-agent"
    }

    fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    fn review(
        &self,
        _request: &AgentReviewRequest,
        cancellation: &CancellationToken,
    ) -> Result<AgentReviewResponse, AgentAdapterError> {
        if cancellation.is_cancelled() {
            return Err(AgentAdapterError::Timeout);
        }
        let behavior = self
            .behavior
            .lock()
            .map_err(|_| AgentAdapterError::Crashed)?
            .clone();
        match behavior {
            MockAgentBehavior::Response(response) => Ok(response),
            MockAgentBehavior::Raw(raw) => {
                from_json(&raw).map_err(|_| AgentAdapterError::MalformedOutput)
            }
            MockAgentBehavior::Timeout => Err(AgentAdapterError::Timeout),
            MockAgentBehavior::Crash => Err(AgentAdapterError::Crashed),
            MockAgentBehavior::Unavailable => Err(AgentAdapterError::Unavailable),
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use surfacecheck_core::{
        AgentCategory, AgentFinding, ConsentRecord, Provenance, ProvenanceKind, Severity,
        SCHEMA_VERSION,
    };

    fn provenance() -> Provenance {
        Provenance {
            kind: ProvenanceKind::AgentReview,
            producer: "test".into(),
            producer_version: "0.1".into(),
            producer_commit: "fixture".into(),
            tool_versions: Vec::new(),
        }
    }

    fn request() -> AgentReviewRequest {
        AgentReviewRequest {
            schema_version: SCHEMA_VERSION,
            review_id: "review-1".into(),
            capture_id: "capture-1".into(),
            prompt: "Describe visible concerns.".into(),
            evidence: vec![EvidenceRef {
                capture_id: "capture-1".into(),
                content_sha256: "0".repeat(64),
                region: surfacecheck_core::EvidenceRegion {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                },
            }],
            consent: ConsentRecord {
                acknowledged: true,
                local_only: true,
                disclosure: "The selected local evidence will be reviewed by an agent.".into(),
            },
            provenance: provenance(),
        }
    }

    fn context() -> AgentReviewContext {
        AgentReviewContext::new(vec![CaptureContext {
            capture_id: "capture-1".into(),
            dimensions: Dimensions {
                width: 10,
                height: 10,
            },
            content_sha256: "0".repeat(64),
        }])
        .expect("context")
    }

    fn response() -> AgentReviewResponse {
        AgentReviewResponse {
            schema_version: SCHEMA_VERSION,
            review_id: "review-1".into(),
            status: OperationStatus::Success,
            findings: vec![AgentFinding {
                finding_id: "finding-1".into(),
                category: AgentCategory::Layout,
                severity: Severity::Low,
                evidence: request().evidence,
                explanation: "A visible alignment concern.".into(),
                confidence: 0.7,
                suggested_next_action: "Inspect the spacing.".into(),
            }],
            error: None,
            provenance: provenance(),
        }
    }

    #[test]
    fn explicit_consent_and_structured_success_are_required() {
        let adapter = Arc::new(MockAgentAdapter::new(MockAgentBehavior::Response(
            response(),
        )));
        let coordinator = ReviewCoordinator::new(Some(adapter));
        let output = coordinator
            .review(&request(), &context(), &CancellationToken::new())
            .expect("review");
        assert_eq!(output.findings.len(), 1);
    }

    #[test]
    fn disabled_and_incompatible_adapters_are_honest() {
        let coordinator = ReviewCoordinator::new(None);
        assert_eq!(
            coordinator.review(&request(), &context(), &CancellationToken::new()),
            Err(AgentReviewError::AdapterUnavailable)
        );
        let adapter = Arc::new(
            MockAgentAdapter::new(MockAgentBehavior::Response(response())).with_protocol_version(2),
        );
        let coordinator = ReviewCoordinator::new(Some(adapter));
        assert_eq!(
            coordinator.review(&request(), &context(), &CancellationToken::new()),
            Err(AgentReviewError::AdapterIncompatible)
        );
    }

    #[test]
    fn malformed_and_hostile_findings_are_rejected() {
        let adapter = Arc::new(MockAgentAdapter::new(MockAgentBehavior::Raw(
            br#"{"schemaVersion":1,"reviewId":"review-1","status":"success","findings":[],"error":null,"provenance":{"kind":"agent_review","producer":"x","producerVersion":"x","producerCommit":"x","toolVersions":[]},"extra":true}"#.to_vec(),
        )));
        let coordinator = ReviewCoordinator::new(Some(adapter));
        assert_eq!(
            coordinator.review(&request(), &context(), &CancellationToken::new()),
            Err(AgentReviewError::MalformedOutput)
        );

        let mut invalid = response();
        invalid.findings[0].evidence[0].region.x = 9;
        invalid.findings[0].evidence[0].region.width = 2;
        let adapter = Arc::new(MockAgentAdapter::new(MockAgentBehavior::Response(invalid)));
        let coordinator = ReviewCoordinator::new(Some(adapter));
        assert_eq!(
            coordinator.review(&request(), &context(), &CancellationToken::new()),
            Err(AgentReviewError::FindingOutsideEvidence)
        );
    }

    #[test]
    fn cancellation_and_failure_statuses_are_distinct() {
        let token = CancellationToken::new();
        token.cancel();
        let adapter = Arc::new(MockAgentAdapter::new(MockAgentBehavior::Response(
            response(),
        )));
        assert_eq!(
            ReviewCoordinator::new(Some(adapter)).review(&request(), &context(), &token),
            Err(AgentReviewError::Cancelled)
        );
        let adapter = Arc::new(MockAgentAdapter::new(MockAgentBehavior::Timeout));
        assert_eq!(
            ReviewCoordinator::new(Some(adapter)).review(
                &request(),
                &context(),
                &CancellationToken::new()
            ),
            Err(AgentReviewError::Timeout)
        );
    }
}
