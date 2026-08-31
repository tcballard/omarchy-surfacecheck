use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

/// The only contract version emitted by the v0.1 implementation.
pub const SCHEMA_VERSION: u16 = 1;

pub const MAX_JSON_FRAME_BYTES: usize = 1_048_576;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_VERSION_BYTES: usize = 128;
pub const MAX_PROVENANCE_TEXT_BYTES: usize = 256;
pub const MAX_CAPTURE_COUNT: usize = 32;
pub const MAX_STORED_SESSIONS: usize = 50;
pub const MAX_EVIDENCE_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;
pub const MAX_DECODED_RGBA_BYTES: u64 = 400 * 1024 * 1024;
pub const MAX_FINDINGS: usize = 256;
pub const MAX_AGENT_FINDINGS: usize = 128;
pub const MAX_EVIDENCE_REFS: usize = 16;
pub const MAX_AGENT_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_AGENT_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_SUGGESTED_ACTION_BYTES: usize = 4 * 1024;
pub const MAX_ERROR_MESSAGE_BYTES: usize = 4 * 1024;
pub const MAX_ARCHIVE_PATH_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl Error for ValidationError {}

#[derive(Debug)]
pub enum ContractError {
    Json(serde_json::Error),
    Validation(ValidationError),
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid JSON: {error}"),
            Self::Validation(error) => error.fmt(f),
        }
    }
}

impl Error for ContractError {}

impl From<serde_json::Error> for ContractError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Implemented by every public contract record.
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

/// Parse and validate a bounded versioned record.
pub fn from_json<T>(input: &[u8]) -> Result<T, ContractError>
where
    T: DeserializeOwned + Validate,
{
    if input.len() > MAX_JSON_FRAME_BYTES {
        return Err(ContractError::Validation(ValidationError::new(
            "$",
            format!("JSON frame exceeds {MAX_JSON_FRAME_BYTES} bytes"),
        )));
    }
    let value: T = serde_json::from_slice(input)?;
    value.validate().map_err(ContractError::Validation)?;
    Ok(value)
}

/// Validate and serialize using the declaration order of each record.
///
/// Public records contain no unordered maps.  `serde_json` therefore emits a
/// stable key order, and callers can obtain byte-identical metadata by
/// injecting the same timestamps and identifiers.
pub fn to_canonical_json<T>(value: &T) -> Result<Vec<u8>, ContractError>
where
    T: Serialize + Validate,
{
    value.validate().map_err(ContractError::Validation)?;
    Ok(serde_json::to_vec(value)?)
}

fn validate_schema_version(version: u16) -> Result<(), ValidationError> {
    if version != SCHEMA_VERSION {
        return Err(ValidationError::new(
            "schemaVersion",
            format!("unsupported schema version {version}; expected {SCHEMA_VERSION}"),
        ));
    }
    Ok(())
}

fn validate_text(
    value: &str,
    field: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), ValidationError> {
    let length = value.len();
    if !allow_empty && value.is_empty() {
        return Err(ValidationError::new(field, "must not be empty"));
    }
    if length > max_bytes {
        return Err(ValidationError::new(
            field,
            format!("must be at most {max_bytes} UTF-8 bytes"),
        ));
    }
    if value.chars().any(|character| {
        character == '\0' || character.is_control() && character != '\n' && character != '\t'
    }) {
        return Err(ValidationError::new(
            field,
            "contains a forbidden control character",
        ));
    }
    Ok(())
}

fn validate_id(value: &str, field: &str) -> Result<(), ValidationError> {
    validate_text(value, field, MAX_ID_BYTES, false)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ValidationError::new(
            field,
            "must contain only ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str, field: &str) -> Result<(), ValidationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ValidationError::new(
            field,
            "must be exactly 64 lowercase hexadecimal characters",
        ));
    }
    Ok(())
}

fn validate_nonempty_bounded<T>(
    items: &[T],
    field: &str,
    max: usize,
) -> Result<(), ValidationError> {
    if items.is_empty() {
        return Err(ValidationError::new(field, "must not be empty"));
    }
    if items.len() > max {
        return Err(ValidationError::new(
            field,
            format!("must contain at most {max} entries"),
        ));
    }
    Ok(())
}

fn validate_bounded<T>(items: &[T], field: &str, max: usize) -> Result<(), ValidationError> {
    if items.len() > max {
        return Err(ValidationError::new(
            field,
            format!("must contain at most {max} entries"),
        ));
    }
    Ok(())
}

fn validate_finite(value: f64, field: &str) -> Result<(), ValidationError> {
    if !value.is_finite() {
        return Err(ValidationError::new(field, "must be finite"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureType {
    Window,
    Region,
    Application,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Success,
    Unavailable,
    MissingTool,
    Cancelled,
    Invalid,
    Busy,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    Grim,
    Slurp,
    Hyprctl,
    Surfacecheck,
    AgentAdapter,
    PremonitionAdapter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    LocalCapture,
    DeterministicReview,
    AgentReview,
    Comparison,
    Export,
    Handoff,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeterministicCategory {
    EmptyCapture,
    CorruptCapture,
    UnexpectedDimensions,
    ScaleInconsistency,
    BoundaryContact,
    ContrastMeasurement,
    PixelDifference,
    MissingEvidence,
    DuplicateCapture,
    StaleCapture,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCategory {
    Layout,
    Typography,
    Contrast,
    Clipping,
    Interaction,
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSource {
    Deterministic,
    Agent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    RuntimeUnavailable,
    MissingCaptureTool,
    SelectionCancelled,
    InvalidRequest,
    InvalidEvidence,
    OperationBusy,
    OperationTimeout,
    ToolFailed,
    StorageLimit,
    AdapterUnavailable,
    AdapterIncompatible,
    MalformedAgentOutput,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliCommand {
    Status,
    CaptureWindow,
    CaptureRegion,
    CaptureApplication,
    Review,
    Compare,
    Export,
    HandoffPremonition,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Validate for Dimensions {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(ValidationError::new(
                "dimensions",
                "width and height must be positive",
            ));
        }
        if self.width > MAX_IMAGE_DIMENSION || self.height > MAX_IMAGE_DIMENSION {
            return Err(ValidationError::new(
                "dimensions",
                format!("width and height must be at most {MAX_IMAGE_DIMENSION}"),
            ));
        }
        let pixels = u64::from(self.width) * u64::from(self.height);
        if pixels > MAX_IMAGE_PIXELS {
            return Err(ValidationError::new(
                "dimensions",
                format!("pixel count must be at most {MAX_IMAGE_PIXELS}"),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scale {
    pub x: f64,
    pub y: f64,
}

impl Validate for Scale {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_finite(self.x, "scale.x")?;
        validate_finite(self.y, "scale.y")?;
        if !(0.1..=16.0).contains(&self.x) || !(0.1..=16.0).contains(&self.y) {
            return Err(ValidationError::new(
                "scale",
                "values must be between 0.1 and 16.0",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Region {
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}

impl Validate for Region {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(ValidationError::new(
                "region",
                "width and height must be positive",
            ));
        }
        if self.width > MAX_IMAGE_DIMENSION || self.height > MAX_IMAGE_DIMENSION {
            return Err(ValidationError::new(
                "region",
                "width and height exceed the image bound",
            ));
        }
        if u64::from(self.width) * u64::from(self.height) > MAX_IMAGE_PIXELS {
            return Err(ValidationError::new(
                "region",
                "pixel count exceeds the image bound",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl EvidenceRegion {
    fn validate_against(
        &self,
        dimensions: &Dimensions,
        field: &str,
    ) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(ValidationError::new(
                field,
                "width and height must be positive",
            ));
        }
        let right = u64::from(self.x) + u64::from(self.width);
        let bottom = u64::from(self.y) + u64::from(self.height);
        if right > u64::from(dimensions.width) || bottom > u64::from(dimensions.height) {
            return Err(ValidationError::new(
                field,
                "region lies outside the capture bounds",
            ));
        }
        Ok(())
    }
}

impl Validate for EvidenceRegion {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(ValidationError::new(
                "region",
                "width and height must be positive",
            ));
        }
        if self.width > MAX_IMAGE_DIMENSION || self.height > MAX_IMAGE_DIMENSION {
            return Err(ValidationError::new(
                "region",
                "width and height exceed the image bound",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolVersion {
    pub name: ToolName,
    pub version: String,
}

impl Validate for ToolVersion {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            &self.version,
            "toolVersion.version",
            MAX_TOOL_VERSION_BYTES,
            false,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationIdentity {
    /// Deliberately an alias, never a raw title, class, URL or path.
    pub redacted_alias: String,
}

impl Validate for ApplicationIdentity {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            &self.redacted_alias,
            "application.redactedAlias",
            MAX_PROVENANCE_TEXT_BYTES,
            false,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provenance {
    pub kind: ProvenanceKind,
    pub producer: String,
    pub producer_version: String,
    pub producer_commit: String,
    pub tool_versions: Vec<ToolVersion>,
}

impl Validate for Provenance {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            &self.producer,
            "provenance.producer",
            MAX_PROVENANCE_TEXT_BYTES,
            false,
        )?;
        validate_text(
            &self.producer_version,
            "provenance.producerVersion",
            MAX_PROVENANCE_TEXT_BYTES,
            false,
        )?;
        validate_text(
            &self.producer_commit,
            "provenance.producerCommit",
            MAX_PROVENANCE_TEXT_BYTES,
            false,
        )?;
        validate_bounded(&self.tool_versions, "provenance.toolVersions", 16)?;
        for tool in &self.tool_versions {
            tool.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageEvidence {
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl Validate for ImageEvidence {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            &self.relative_path,
            "image.relativePath",
            MAX_ARCHIVE_PATH_BYTES,
            false,
        )?;
        if self.relative_path.starts_with('/')
            || self.relative_path.contains('\\')
            || self
                .relative_path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(ValidationError::new(
                "image.relativePath",
                "must be a normalized relative object path",
            ));
        }
        if self.bytes == 0 || self.bytes > MAX_IMAGE_BYTES {
            return Err(ValidationError::new(
                "image.bytes",
                format!("must be between 1 and {MAX_IMAGE_BYTES}"),
            ));
        }
        validate_sha256(&self.sha256, "image.sha256")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRef {
    pub capture_id: String,
    pub region: EvidenceRegion,
}

impl Validate for EvidenceRef {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.capture_id, "evidence.captureId")?;
        self.region.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRecord {
    pub capture_id: String,
    pub capture_type: CaptureType,
    pub captured_at: u64,
    pub image: ImageEvidence,
    pub dimensions: Dimensions,
    pub scale: Scale,
    pub selection: Option<Region>,
    pub tool_versions: Vec<ToolVersion>,
    pub application: Option<ApplicationIdentity>,
}

impl Validate for CaptureRecord {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.capture_id, "captureId")?;
        self.image.validate()?;
        self.dimensions.validate()?;
        self.scale.validate()?;
        if let Some(selection) = &self.selection {
            selection.validate()?;
        }
        validate_bounded(&self.tool_versions, "toolVersions", 16)?;
        for tool in &self.tool_versions {
            tool.validate()?;
        }
        if let Some(application) = &self.application {
            application.validate()?;
        }
        let decoded_bytes = u64::from(self.dimensions.width)
            .checked_mul(u64::from(self.dimensions.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| ValidationError::new("dimensions", "decoded size overflows"))?;
        if decoded_bytes > MAX_DECODED_RGBA_BYTES {
            return Err(ValidationError::new(
                "dimensions",
                "decoded RGBA size exceeds the image bound",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeterministicFinding {
    pub finding_id: String,
    pub category: DeterministicCategory,
    pub severity: Severity,
    pub evidence: Vec<EvidenceRef>,
    pub explanation: String,
    pub code: String,
    pub measurement: Option<f64>,
}

impl Validate for DeterministicFinding {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.finding_id, "findingId")?;
        validate_id(&self.code, "code")?;
        validate_nonempty_bounded(&self.evidence, "evidence", MAX_EVIDENCE_REFS)?;
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        validate_text(&self.explanation, "explanation", MAX_TEXT_BYTES, false)?;
        if let Some(measurement) = self.measurement {
            validate_finite(measurement, "measurement")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFinding {
    pub finding_id: String,
    pub category: AgentCategory,
    pub severity: Severity,
    pub evidence: Vec<EvidenceRef>,
    pub explanation: String,
    pub confidence: f64,
    pub suggested_next_action: String,
}

impl Validate for AgentFinding {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.finding_id, "findingId")?;
        validate_nonempty_bounded(&self.evidence, "evidence", MAX_EVIDENCE_REFS)?;
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        validate_text(&self.explanation, "explanation", MAX_TEXT_BYTES, false)?;
        validate_finite(self.confidence, "confidence")?;
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(ValidationError::new(
                "confidence",
                "must be between 0.0 and 1.0",
            ));
        }
        validate_text(
            &self.suggested_next_action,
            "suggestedNextAction",
            MAX_SUGGESTED_ACTION_BYTES,
            false,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComparisonRecord {
    pub comparison_id: String,
    pub before_capture_id: String,
    pub after_capture_id: String,
    pub dimensions: Dimensions,
    pub same_scale: bool,
    pub changed_pixels: u64,
    pub changed_fraction: f64,
    pub mean_absolute_difference: f64,
    pub perceptual_distance: Option<f64>,
}

impl Validate for ComparisonRecord {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.comparison_id, "comparisonId")?;
        validate_id(&self.before_capture_id, "beforeCaptureId")?;
        validate_id(&self.after_capture_id, "afterCaptureId")?;
        if self.before_capture_id == self.after_capture_id {
            return Err(ValidationError::new(
                "comparison",
                "before and after captures must differ",
            ));
        }
        self.dimensions.validate()?;
        let pixels = u64::from(self.dimensions.width) * u64::from(self.dimensions.height);
        if self.changed_pixels > pixels {
            return Err(ValidationError::new(
                "changedPixels",
                "cannot exceed total pixels",
            ));
        }
        validate_finite(self.changed_fraction, "changedFraction")?;
        if !(0.0..=1.0).contains(&self.changed_fraction) {
            return Err(ValidationError::new(
                "changedFraction",
                "must be between 0.0 and 1.0",
            ));
        }
        validate_finite(self.mean_absolute_difference, "meanAbsoluteDifference")?;
        if !(0.0..=1.0).contains(&self.mean_absolute_difference) {
            return Err(ValidationError::new(
                "meanAbsoluteDifference",
                "must be between 0.0 and 1.0",
            ));
        }
        if let Some(distance) = self.perceptual_distance {
            validate_finite(distance, "perceptualDistance")?;
            if distance < 0.0 {
                return Err(ValidationError::new(
                    "perceptualDistance",
                    "must not be negative",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeforeAfter {
    pub before_capture_id: String,
    pub after_capture_id: String,
    pub comparison_id: String,
}

impl Validate for BeforeAfter {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.before_capture_id, "beforeAfter.beforeCaptureId")?;
        validate_id(&self.after_capture_id, "beforeAfter.afterCaptureId")?;
        validate_id(&self.comparison_id, "beforeAfter.comparisonId")?;
        if self.before_capture_id == self.after_capture_id {
            return Err(ValidationError::new(
                "beforeAfter",
                "before and after captures must differ",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceManifest {
    pub schema_version: u16,
    pub session_id: String,
    pub created_at: u64,
    pub captures: Vec<CaptureRecord>,
    pub user_note: Option<String>,
    pub deterministic_findings: Vec<DeterministicFinding>,
    pub agent_findings: Vec<AgentFinding>,
    pub comparison: Option<ComparisonRecord>,
    pub before_after: Option<BeforeAfter>,
    pub provenance: Provenance,
}

impl Validate for EvidenceManifest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.session_id, "sessionId")?;
        validate_nonempty_bounded(&self.captures, "captures", MAX_CAPTURE_COUNT)?;
        for capture in &self.captures {
            capture.validate()?;
        }
        if let Some(note) = &self.user_note {
            validate_text(note, "userNote", MAX_TEXT_BYTES, true)?;
        }
        validate_bounded(
            &self.deterministic_findings,
            "deterministicFindings",
            MAX_FINDINGS,
        )?;
        for finding in &self.deterministic_findings {
            finding.validate()?;
        }
        validate_bounded(&self.agent_findings, "agentFindings", MAX_AGENT_FINDINGS)?;
        for finding in &self.agent_findings {
            finding.validate()?;
        }
        self.provenance.validate()?;
        if let Some(comparison) = &self.comparison {
            comparison.validate()?;
        }
        if let Some(before_after) = &self.before_after {
            before_after.validate()?;
            if self
                .comparison
                .as_ref()
                .map(|comparison| comparison.comparison_id.as_str())
                != Some(before_after.comparison_id.as_str())
            {
                return Err(ValidationError::new(
                    "beforeAfter.comparisonId",
                    "must refer to the manifest comparison",
                ));
            }
        }

        for (index, capture) in self.captures.iter().enumerate() {
            if self.captures[..index]
                .iter()
                .any(|previous| previous.capture_id == capture.capture_id)
            {
                return Err(ValidationError::new(
                    "captures",
                    "capture IDs must be unique",
                ));
            }
        }
        self.validate_finding_refs()?;
        if let Some(comparison) = &self.comparison {
            if !self.has_capture(&comparison.before_capture_id)
                || !self.has_capture(&comparison.after_capture_id)
            {
                return Err(ValidationError::new(
                    "comparison",
                    "references a missing capture",
                ));
            }
            let before = self
                .capture(&comparison.before_capture_id)
                .expect("checked above");
            let after = self
                .capture(&comparison.after_capture_id)
                .expect("checked above");
            if before.dimensions != comparison.dimensions
                || after.dimensions != comparison.dimensions
            {
                return Err(ValidationError::new(
                    "comparison.dimensions",
                    "must match both captures",
                ));
            }
            if before.scale != after.scale && comparison.same_scale {
                return Err(ValidationError::new(
                    "comparison.sameScale",
                    "cannot be true for different scales",
                ));
            }
        }
        if let Some(before_after) = &self.before_after {
            if !self.has_capture(&before_after.before_capture_id)
                || !self.has_capture(&before_after.after_capture_id)
            {
                return Err(ValidationError::new(
                    "beforeAfter",
                    "references a missing capture",
                ));
            }
        }
        Ok(())
    }
}

impl EvidenceManifest {
    fn capture(&self, capture_id: &str) -> Option<&CaptureRecord> {
        self.captures
            .iter()
            .find(|capture| capture.capture_id == capture_id)
    }

    fn has_capture(&self, capture_id: &str) -> bool {
        self.capture(capture_id).is_some()
    }

    fn validate_finding_refs(&self) -> Result<(), ValidationError> {
        for finding in &self.deterministic_findings {
            for evidence in &finding.evidence {
                self.validate_evidence_ref(evidence)?;
            }
        }
        for finding in &self.agent_findings {
            for evidence in &finding.evidence {
                self.validate_evidence_ref(evidence)?;
            }
        }
        Ok(())
    }

    fn validate_evidence_ref(&self, evidence: &EvidenceRef) -> Result<(), ValidationError> {
        let capture = self.capture(&evidence.capture_id).ok_or_else(|| {
            ValidationError::new("evidence.captureId", "references a missing capture")
        })?;
        evidence
            .region
            .validate_against(&capture.dimensions, "evidence.region")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmptyRequest {}

impl Validate for EmptyRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureWindowRequest {
    pub user_note: Option<String>,
}

impl Validate for CaptureWindowRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(note) = &self.user_note {
            validate_text(note, "userNote", MAX_TEXT_BYTES, true)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRegionRequest {
    pub region: Region,
    pub user_note: Option<String>,
}

impl Validate for CaptureRegionRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        self.region.validate()?;
        if let Some(note) = &self.user_note {
            validate_text(note, "userNote", MAX_TEXT_BYTES, true)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureApplicationRequest {
    pub application_alias: String,
    pub user_note: Option<String>,
}

impl Validate for CaptureApplicationRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            &self.application_alias,
            "applicationAlias",
            MAX_PROVENANCE_TEXT_BYTES,
            false,
        )?;
        if let Some(note) = &self.user_note {
            validate_text(note, "userNote", MAX_TEXT_BYTES, true)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewRequest {
    pub capture_id: String,
    pub disclose_agent: bool,
}

impl Validate for ReviewRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.capture_id, "captureId")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompareRequest {
    pub before_capture_id: String,
    pub after_capture_id: String,
}

impl Validate for CompareRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.before_capture_id, "beforeCaptureId")?;
        validate_id(&self.after_capture_id, "afterCaptureId")?;
        if self.before_capture_id == self.after_capture_id {
            return Err(ValidationError::new(
                "compare",
                "before and after captures must differ",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    pub session_id: String,
}

impl Validate for ExportRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.session_id, "sessionId")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelRequest {
    pub operation_id: String,
    pub generation: u64,
}

impl Validate for CancelRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.operation_id, "operationId")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CliRequest<T> {
    pub schema_version: u16,
    pub request_id: String,
    pub command: CliCommand,
    pub payload: T,
}

impl<T: Validate> Validate for CliRequest<T> {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.request_id, "requestId")?;
        self.payload.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl Validate for ErrorEnvelope {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            &self.message,
            "error.message",
            MAX_ERROR_MESSAGE_BYTES,
            false,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CliResponse<T> {
    pub schema_version: u16,
    pub request_id: String,
    pub status: OperationStatus,
    pub result: Option<T>,
    pub error: Option<ErrorEnvelope>,
}

impl<T: Validate> Validate for CliResponse<T> {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.request_id, "requestId")?;
        match (&self.status, &self.result, &self.error) {
            (OperationStatus::Success, Some(result), None) => result.validate()?,
            (OperationStatus::Success, _, _) => {
                return Err(ValidationError::new(
                    "response",
                    "success requires result and no error",
                ))
            }
            (_, None, Some(error)) => error.validate()?,
            (_, _, _) => {
                return Err(ValidationError::new(
                    "response",
                    "non-success requires error and no result",
                ))
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentReviewRequest {
    pub schema_version: u16,
    pub review_id: String,
    pub capture_id: String,
    pub prompt: String,
    pub evidence: Vec<EvidenceRef>,
    pub provenance: Provenance,
}

impl Validate for AgentReviewRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.review_id, "reviewId")?;
        validate_id(&self.capture_id, "captureId")?;
        validate_text(&self.prompt, "prompt", MAX_AGENT_PROMPT_BYTES, false)?;
        validate_nonempty_bounded(&self.evidence, "evidence", MAX_EVIDENCE_REFS)?;
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        self.provenance.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentReviewResponse {
    pub schema_version: u16,
    pub review_id: String,
    pub status: OperationStatus,
    pub findings: Vec<AgentFinding>,
    pub error: Option<ErrorEnvelope>,
    pub provenance: Provenance,
}

impl Validate for AgentReviewResponse {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.review_id, "reviewId")?;
        validate_bounded(&self.findings, "findings", MAX_AGENT_FINDINGS)?;
        for finding in &self.findings {
            finding.validate()?;
        }
        self.provenance.validate()?;
        match (&self.status, &self.error) {
            (OperationStatus::Success, None) => Ok(()),
            (OperationStatus::Success, Some(_)) => Err(ValidationError::new(
                "response",
                "success cannot carry an error",
            )),
            (_, Some(error)) => error.validate(),
            (_, None) => Err(ValidationError::new(
                "response",
                "non-success requires an error",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefectEnvelope {
    pub defect_id: String,
    pub finding_id: String,
    pub finding_source: FindingSource,
    pub capture_id: String,
    pub category: AgentCategory,
    pub severity: Severity,
    pub explanation: String,
    pub evidence: Vec<EvidenceRef>,
    pub suggested_next_action: String,
    pub provenance: Provenance,
}

impl Validate for DefectEnvelope {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.defect_id, "defectId")?;
        validate_id(&self.finding_id, "findingId")?;
        validate_id(&self.capture_id, "captureId")?;
        validate_text(&self.explanation, "explanation", MAX_TEXT_BYTES, false)?;
        validate_nonempty_bounded(&self.evidence, "evidence", MAX_EVIDENCE_REFS)?;
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        validate_text(
            &self.suggested_next_action,
            "suggestedNextAction",
            MAX_SUGGESTED_ACTION_BYTES,
            false,
        )?;
        self.provenance.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PremonitionHandoffRequest {
    pub schema_version: u16,
    pub handoff_id: String,
    pub adapter_protocol_version: u16,
    pub defect: DefectEnvelope,
}

impl Validate for PremonitionHandoffRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.handoff_id, "handoffId")?;
        if self.adapter_protocol_version == 0 {
            return Err(ValidationError::new(
                "adapterProtocolVersion",
                "must be positive",
            ));
        }
        self.defect.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PremonitionHandoffResponse {
    pub schema_version: u16,
    pub handoff_id: String,
    pub status: OperationStatus,
    pub external_reference: Option<String>,
    pub error: Option<ErrorEnvelope>,
}

impl Validate for PremonitionHandoffResponse {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema_version(self.schema_version)?;
        validate_id(&self.handoff_id, "handoffId")?;
        if let Some(reference) = &self.external_reference {
            validate_text(reference, "externalReference", MAX_ID_BYTES, false)?;
        }
        match (&self.status, &self.external_reference, &self.error) {
            (OperationStatus::Success, Some(_), None) => Ok(()),
            (OperationStatus::Success, _, _) => Err(ValidationError::new(
                "response",
                "success requires an external reference and no error",
            )),
            (_, None, Some(error)) => error.validate(),
            (_, _, _) => Err(ValidationError::new(
                "response",
                "non-success requires an error and no external reference",
            )),
        }
    }
}
