//! Strict, bounded command parsing and the schemaVersion 1 JSON facade.
//!
//! The CLI is intentionally a small client. Evidence mutation belongs to
//! `surfacecheckd`; when the daemon is absent, capability probing and direct
//! capture remain available but are explicitly marked `stored: false`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use surfacecheck_capture::{CaptureEngine, CaptureMode, DirectCommandRunner};
use surfacecheck_core::{
    json_nesting_within_limit, CaptureApplicationRequest, CaptureRegionRequest, CaptureType,
    CaptureWindowRequest, CliCommand, CompareRequest, ConsentRecord, ErrorCode,
    HandoffFindingRequest, OperationStatus, ReviewRequest, Scale, SelectBeforeAfterRequest,
    MAX_ERROR_MESSAGE_BYTES, MAX_ID_BYTES, MAX_JSON_FRAME_BYTES, MAX_TEXT_BYTES, SCHEMA_VERSION,
};
use surfacecheck_service::{read_frame, write_frame, RuntimePaths};

const DEFAULT_SESSION_ID: &str = "session-current";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Status,
    Service,
    CaptureWindow {
        session_id: Option<String>,
        note: Option<String>,
    },
    CaptureRegion {
        session_id: Option<String>,
        note: Option<String>,
    },
    CaptureApplication {
        address: String,
        alias: Option<String>,
        session_id: Option<String>,
        note: Option<String>,
    },
    Review {
        session_id: Option<String>,
        capture_id: String,
        agent: bool,
        consent: bool,
    },
    Compare {
        session_id: Option<String>,
        before_id: String,
        after_id: String,
    },
    Annotate {
        session_id: String,
        note: String,
    },
    SelectBeforeAfter {
        session_id: String,
        before_id: String,
        after_id: String,
    },
    Export {
        session_id: String,
    },
    HandoffPremonition {
        session_id: String,
        finding_id: String,
        consent: bool,
    },
    Cancel {
        operation_id: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub command: Command,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JsonEnvelope {
    pub schema_version: u16,
    pub request_id: String,
    pub status: OperationStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JsonError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolStatus {
    tool: surfacecheck_core::ToolName,
    status: OperationStatus,
    version: Option<String>,
    error: Option<ErrorCode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResult {
    daemon: OperationStatus,
    runtime_configured: bool,
    service_socket_present: bool,
    tools: Vec<ToolStatus>,
    agent: OperationStatus,
    handoff: OperationStatus,
    max_captures: usize,
    max_bundle_bytes: u64,
    current_operation: Option<surfacecheck_service::BusyOperation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureResult {
    session_id: Option<String>,
    capture_id: String,
    capture_type: CaptureType,
    width: u32,
    height: u32,
    scale: Scale,
    sha256: String,
    stale: bool,
    stored: bool,
}

#[derive(Debug, Default)]
struct Options {
    json_count: usize,
    agent: bool,
    consent_local: bool,
    consent_external: bool,
    session: Option<String>,
    note: Option<String>,
    alias: Option<String>,
}

pub fn parse_args(args: &[String]) -> Result<ParsedCommand, String> {
    if args.is_empty() {
        return Err("a command is required".into());
    }
    let mut positional = Vec::new();
    let mut options = Options::default();
    let mut index = 0usize;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--json" => options.json_count += 1,
            "--agent" => {
                if options.agent {
                    return Err("--agent may be supplied only once".into());
                }
                options.agent = true;
            }
            "--consent-local" => {
                if options.consent_local || options.consent_external {
                    return Err("only one consent flag may be supplied".into());
                }
                options.consent_local = true;
            }
            "--consent-external" => {
                if options.consent_external || options.consent_local {
                    return Err("only one consent flag may be supplied".into());
                }
                options.consent_external = true;
            }
            "--session" => {
                options.session = Some(next_option_value(args, &mut index, "--session")?);
            }
            "--note" => {
                options.note = Some(next_option_value(args, &mut index, "--note")?);
            }
            "--alias" => {
                options.alias = Some(next_option_value(args, &mut index, "--alias")?);
            }
            value if value.starts_with('-') => return Err("unknown option".into()),
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }
    if options.json_count != 1 {
        return Err("exactly one --json option is required".into());
    }
    validate_options(&options)?;
    let words = positional.iter().map(String::as_str).collect::<Vec<_>>();
    let command = match words.as_slice() {
        ["status"] => {
            reject_unused(&options, false, false, false, false, false)?;
            Command::Status
        }
        ["service"] => {
            reject_unused(&options, false, false, false, false, false)?;
            Command::Service
        }
        ["capture", "window"] => {
            reject_unused(&options, true, true, false, false, false)?;
            Command::CaptureWindow {
                session_id: options.session,
                note: options.note,
            }
        }
        ["capture", "region"] => {
            reject_unused(&options, true, true, false, false, false)?;
            Command::CaptureRegion {
                session_id: options.session,
                note: options.note,
            }
        }
        ["capture", "application", address] if valid_address(address) => {
            reject_unused(&options, true, true, true, false, false)?;
            Command::CaptureApplication {
                address: (*address).to_owned(),
                alias: options.alias,
                session_id: options.session,
                note: options.note,
            }
        }
        ["review", capture_id] if valid_id(capture_id) => {
            reject_unused(&options, true, false, false, true, false)?;
            Command::Review {
                session_id: options.session,
                capture_id: (*capture_id).to_owned(),
                agent: options.agent,
                consent: options.consent_local,
            }
        }
        ["compare", before_id, after_id]
            if valid_id(before_id) && valid_id(after_id) && before_id != after_id =>
        {
            reject_unused(&options, true, false, false, false, false)?;
            Command::Compare {
                session_id: options.session,
                before_id: (*before_id).to_owned(),
                after_id: (*after_id).to_owned(),
            }
        }
        ["annotate", session_id] if valid_id(session_id) && options.note.is_some() => {
            reject_unused(&options, false, true, false, false, false)?;
            Command::Annotate {
                session_id: (*session_id).to_owned(),
                note: options.note.expect("checked above"),
            }
        }
        ["select-before-after", session_id, before_id, after_id]
            if valid_id(session_id)
                && valid_id(before_id)
                && valid_id(after_id)
                && before_id != after_id =>
        {
            reject_unused(&options, false, false, false, false, false)?;
            Command::SelectBeforeAfter {
                session_id: (*session_id).to_owned(),
                before_id: (*before_id).to_owned(),
                after_id: (*after_id).to_owned(),
            }
        }
        ["export", session_id] if valid_id(session_id) => {
            reject_unused(&options, false, false, false, false, false)?;
            Command::Export {
                session_id: (*session_id).to_owned(),
            }
        }
        ["handoff", "premonition", finding_id] if valid_id(finding_id) => {
            reject_unused(&options, true, false, false, false, true)?;
            Command::HandoffPremonition {
                session_id: options
                    .session
                    .unwrap_or_else(|| DEFAULT_SESSION_ID.to_owned()),
                finding_id: (*finding_id).to_owned(),
                consent: options.consent_external,
            }
        }
        ["cancel", operation_id, generation] if valid_id(operation_id) => {
            reject_unused(&options, false, false, false, false, false)?;
            let generation = generation
                .parse::<u64>()
                .map_err(|_| "generation must be an unsigned integer")?;
            Command::Cancel {
                operation_id: (*operation_id).to_owned(),
                generation,
            }
        }
        _ => return Err("unknown or malformed command".into()),
    };
    Ok(ParsedCommand {
        request_id: stable_request_id(args),
        command,
    })
}

fn next_option_value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{option} requires a value"))?
        .clone();
    if matches!(
        value.as_str(),
        "--json"
            | "--agent"
            | "--consent-local"
            | "--consent-external"
            | "--session"
            | "--note"
            | "--alias"
    ) {
        return Err(format!("{option} requires a value"));
    }
    Ok(value)
}

fn validate_options(options: &Options) -> Result<(), String> {
    if let Some(session) = &options.session {
        if !valid_id(session) {
            return Err("session must be a safe opaque identifier".into());
        }
    }
    if let Some(note) = &options.note {
        validate_text(note, "note", MAX_TEXT_BYTES)?;
    }
    if let Some(alias) = &options.alias {
        validate_text(alias, "alias", 256)?;
    }
    Ok(())
}

fn reject_unused(
    options: &Options,
    allow_session: bool,
    allow_note: bool,
    allow_alias: bool,
    allow_agent: bool,
    allow_external: bool,
) -> Result<(), String> {
    if !allow_session && options.session.is_some() {
        return Err("--session is not valid for this command".into());
    }
    if !allow_note && options.note.is_some() {
        return Err("--note is not valid for this command".into());
    }
    if !allow_alias && options.alias.is_some() {
        return Err("--alias is not valid for this command".into());
    }
    if !allow_agent && options.agent {
        return Err("--agent is not valid for this command".into());
    }
    if !allow_agent && options.consent_local {
        return Err("--consent-local is not valid for this command".into());
    }
    if options.consent_external && !allow_external {
        return Err("external consent is only valid for a Premonition handoff".into());
    }
    Ok(())
}

fn validate_text(value: &str, field: &str, maximum: usize) -> Result<(), String> {
    if value.len() > maximum
        || value.chars().any(|character| {
            character == '\0' || character.is_control() && character != '\n' && character != '\t'
        })
    {
        return Err(format!(
            "{field} is too long or contains a control character"
        ));
    }
    Ok(())
}

pub fn execute(parsed: &ParsedCommand) -> JsonEnvelope {
    let request_id = parsed.request_id.clone();
    match &parsed.command {
        Command::Status => execute_status(request_id),
        Command::Service => call_service(
            request_id.clone(),
            CliCommand::Service,
            serde_json::json!({}),
        )
        .unwrap_or_else(|error| error.to_envelope(request_id)),
        Command::CaptureWindow { session_id, note } => {
            let payload = CaptureWindowRequest {
                session_id: session_id.clone(),
                user_note: note.clone(),
            };
            capture_command(
                request_id,
                CliCommand::CaptureWindow,
                serde_json::to_value(payload).expect("capture request serializes"),
                CaptureMode::ActiveWindow,
            )
        }
        Command::CaptureRegion { session_id, note } => {
            let payload = CaptureRegionRequest {
                session_id: session_id.clone(),
                region: None,
                user_note: note.clone(),
            };
            capture_command(
                request_id,
                CliCommand::CaptureRegion,
                serde_json::to_value(payload).expect("capture request serializes"),
                CaptureMode::Region,
            )
        }
        Command::CaptureApplication {
            address,
            alias,
            session_id,
            note,
        } => {
            let payload = CaptureApplicationRequest {
                session_id: session_id.clone(),
                application_address: address.clone(),
                application_alias: alias.clone(),
                user_note: note.clone(),
            };
            capture_command(
                request_id,
                CliCommand::CaptureApplication,
                serde_json::to_value(payload).expect("capture request serializes"),
                CaptureMode::Application {
                    address: address.clone(),
                },
            )
        }
        Command::Review {
            session_id,
            capture_id,
            agent,
            consent,
        } => {
            if *agent && !*consent {
                return unavailable(
                    request_id,
                    ErrorCode::InvalidRequest,
                    "explicit --consent-local is required for agent review",
                );
            }
            call_service(
                request_id.clone(),
                CliCommand::Review,
                serde_json::to_value(ReviewRequest {
                    session_id: session_id.clone(),
                    capture_id: capture_id.clone(),
                    disclose_agent: *agent,
                })
                .expect("review request serializes"),
            )
            .unwrap_or_else(|error| error.to_envelope(request_id))
        }
        Command::Compare {
            session_id,
            before_id,
            after_id,
        } => call_service(
            request_id.clone(),
            CliCommand::Compare,
            serde_json::to_value(CompareRequest {
                session_id: session_id.clone(),
                before_capture_id: before_id.clone(),
                after_capture_id: after_id.clone(),
            })
            .expect("comparison request serializes"),
        )
        .unwrap_or_else(|error| error.to_envelope(request_id)),
        Command::Annotate { session_id, note } => call_service(
            request_id.clone(),
            CliCommand::Annotate,
            serde_json::json!({ "sessionId": session_id, "note": note }),
        )
        .unwrap_or_else(|error| error.to_envelope(request_id)),
        Command::SelectBeforeAfter {
            session_id,
            before_id,
            after_id,
        } => call_service(
            request_id.clone(),
            CliCommand::SelectBeforeAfter,
            serde_json::to_value(SelectBeforeAfterRequest {
                session_id: session_id.clone(),
                before_capture_id: before_id.clone(),
                after_capture_id: after_id.clone(),
            })
            .expect("before-after request serializes"),
        )
        .unwrap_or_else(|error| error.to_envelope(request_id)),
        Command::Export { session_id } => call_service(
            request_id.clone(),
            CliCommand::Export,
            serde_json::json!({ "sessionId": session_id }),
        )
        .unwrap_or_else(|error| error.to_envelope(request_id)),
        Command::HandoffPremonition {
            session_id,
            finding_id,
            consent,
        } => {
            if !*consent {
                return unavailable(
                    request_id,
                    ErrorCode::InvalidRequest,
                    "explicit --consent-external is required",
                );
            }
            call_service(
                request_id.clone(),
                CliCommand::HandoffPremonition,
                serde_json::to_value(HandoffFindingRequest {
                    session_id: session_id.clone(),
                    finding_id: finding_id.clone(),
                    consent: ConsentRecord {
                        acknowledged: true,
                        local_only: false,
                        disclosure: "The selected local defect will be handed to Premonition."
                            .into(),
                    },
                })
                .expect("handoff request serializes"),
            )
            .unwrap_or_else(|error| error.to_envelope(request_id))
        }
        Command::Cancel {
            operation_id,
            generation,
        } => call_service(
            request_id.clone(),
            CliCommand::Cancel,
            serde_json::json!({ "operationId": operation_id, "generation": generation }),
        )
        .unwrap_or_else(|error| error.to_envelope(request_id)),
    }
}

fn execute_status(request_id: String) -> JsonEnvelope {
    let runtime_configured = RuntimePaths::from_environment().is_ok();
    let socket_present = RuntimePaths::from_environment()
        .map(|paths| paths.socket.exists())
        .unwrap_or(false);
    if socket_present {
        if let Ok(response) = call_service(
            request_id.clone(),
            CliCommand::Status,
            serde_json::json!({}),
        ) {
            return response;
        }
    }
    local_status(request_id, runtime_configured, socket_present)
}

fn local_status(
    request_id: String,
    runtime_configured: bool,
    socket_present: bool,
) -> JsonEnvelope {
    let mut engine = CaptureEngine::new(DirectCommandRunner);
    engine.timeout = Duration::from_millis(500);
    let tools = engine
        .probe_tools()
        .into_iter()
        .map(|tool| ToolStatus {
            tool: tool.tool,
            status: tool.status,
            version: tool.version,
            error: tool.error,
        })
        .collect();
    success(
        request_id,
        serde_json::to_value(StatusResult {
            daemon: OperationStatus::Unavailable,
            runtime_configured,
            service_socket_present: socket_present,
            tools,
            agent: OperationStatus::Unavailable,
            handoff: OperationStatus::Unavailable,
            max_captures: surfacecheck_core::MAX_CAPTURE_COUNT,
            max_bundle_bytes: surfacecheck_core::MAX_EVIDENCE_BUNDLE_BYTES,
            current_operation: None,
        })
        .unwrap_or_else(|_| serde_json::json!({})),
    )
}

fn capture_command(
    request_id: String,
    command: CliCommand,
    payload: serde_json::Value,
    fallback_mode: CaptureMode,
) -> JsonEnvelope {
    if let Ok(response) = call_service(request_id.clone(), command, payload) {
        return response;
    }
    let socket_present = RuntimePaths::from_environment()
        .map(|paths| paths.socket.exists())
        .unwrap_or(false);
    if socket_present {
        return unavailable(
            request_id,
            ErrorCode::RuntimeUnavailable,
            "SurfaceCheck service is not responding",
        );
    }
    direct_capture(request_id, fallback_mode)
}

fn direct_capture(request_id: String, mode: CaptureMode) -> JsonEnvelope {
    let engine = CaptureEngine::new(DirectCommandRunner);
    match engine.capture(mode) {
        Ok(outcome) => success(
            request_id,
            serde_json::to_value(CaptureResult {
                session_id: None,
                capture_id: stable_capture_id(&outcome.image.sha256, &outcome.region),
                capture_type: outcome.capture_type,
                width: outcome.image.dimensions.width,
                height: outcome.image.dimensions.height,
                scale: capture_scale(),
                sha256: outcome.image.sha256,
                stale: outcome.stale,
                stored: false,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        ),
        Err(error) => unavailable(request_id, error.code, capture_error_message(error.status)),
    }
}

fn capture_scale() -> Scale {
    let parsed = std::env::var("SURFACECHECK_SCALE").ok().and_then(|value| {
        let fields: Vec<_> = value.split(',').collect();
        if fields.len() > 2 {
            return None;
        }
        let x = fields.first()?.trim().parse::<f64>().ok()?;
        let y = fields
            .get(1)
            .map_or(Some(x), |value| value.trim().parse::<f64>().ok())?;
        let scale = Scale { x, y };
        surfacecheck_core::Validate::validate(&scale)
            .ok()
            .map(|_| scale)
    });
    parsed.unwrap_or(Scale { x: 1.0, y: 1.0 })
}

fn capture_error_message(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::MissingTool => "required capture tool is not installed",
        OperationStatus::Cancelled => "selection was cancelled",
        OperationStatus::Timeout => "capture tool timed out",
        OperationStatus::Invalid => "capture evidence was invalid",
        OperationStatus::Unavailable => "compositor is unavailable",
        _ => "capture could not be completed",
    }
}

#[derive(Debug)]
enum ServiceCallError {
    NotConfigured,
    Io,
    Protocol,
}

impl ServiceCallError {
    fn to_envelope(&self, request_id: String) -> JsonEnvelope {
        match self {
            Self::NotConfigured => unavailable(
                request_id,
                ErrorCode::RuntimeUnavailable,
                "SurfaceCheck service is not configured",
            ),
            Self::Io => unavailable(
                request_id,
                ErrorCode::RuntimeUnavailable,
                "SurfaceCheck service is unavailable",
            ),
            Self::Protocol => unavailable(
                request_id,
                ErrorCode::Internal,
                "SurfaceCheck service returned an invalid response",
            ),
        }
    }
}

fn call_service(
    request_id: String,
    command: CliCommand,
    payload: serde_json::Value,
) -> Result<JsonEnvelope, ServiceCallError> {
    #[cfg(not(unix))]
    {
        let _ = (request_id, command, payload);
        return Err(ServiceCallError::NotConfigured);
    }
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let paths =
            RuntimePaths::from_environment().map_err(|_| ServiceCallError::NotConfigured)?;
        if !paths.socket.exists() {
            return Err(ServiceCallError::NotConfigured);
        }
        let mut stream = UnixStream::connect(&paths.socket).map_err(|_| ServiceCallError::Io)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(35)))
            .map_err(|_| ServiceCallError::Io)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(35)))
            .map_err(|_| ServiceCallError::Io)?;
        let request = serde_json::json!({
            "schemaVersion": SCHEMA_VERSION,
            "requestId": request_id,
            "command": command,
            "payload": payload,
        });
        let bytes = serde_json::to_vec(&request).map_err(|_| ServiceCallError::Protocol)?;
        if bytes.len() > MAX_JSON_FRAME_BYTES {
            return Err(ServiceCallError::Protocol);
        }
        write_frame(&mut stream, &bytes).map_err(|_| ServiceCallError::Io)?;
        let response = read_frame(&mut stream).map_err(|_| ServiceCallError::Io)?;
        if response.len() > MAX_JSON_FRAME_BYTES || !json_nesting_within_limit(&response) {
            return Err(ServiceCallError::Protocol);
        }
        let envelope: JsonEnvelope =
            serde_json::from_slice(&response).map_err(|_| ServiceCallError::Protocol)?;
        validate_envelope(&envelope).map_err(|_| ServiceCallError::Protocol)?;
        Ok(envelope)
    }
}

fn validate_envelope(envelope: &JsonEnvelope) -> Result<(), ()> {
    if envelope.schema_version != SCHEMA_VERSION
        || !valid_id(&envelope.request_id)
        || envelope.request_id.len() > MAX_ID_BYTES
    {
        return Err(());
    }
    match (&envelope.status, &envelope.result, &envelope.error) {
        (OperationStatus::Success, Some(_), None) => {}
        (_, None, Some(_)) => {}
        _ => return Err(()),
    }
    if let Some(error) = &envelope.error {
        validate_text(&error.message, "error.message", MAX_ERROR_MESSAGE_BYTES).map_err(|_| ())?;
    }
    Ok(())
}

pub fn render(envelope: &JsonEnvelope) -> Result<Vec<u8>, String> {
    validate_envelope(envelope).map_err(|_| "response shape is invalid".to_owned())?;
    let bytes =
        serde_json::to_vec(envelope).map_err(|_| "could not encode JSON response".to_owned())?;
    if bytes.len() > MAX_JSON_FRAME_BYTES {
        return Err("JSON response exceeds the bounded limit".into());
    }
    Ok(bytes)
}

pub fn run(args: &[String]) -> (Vec<u8>, i32) {
    match parse_args(args) {
        Ok(parsed) => {
            let envelope = execute(&parsed);
            let code = exit_code(envelope.status);
            (
                render(&envelope).unwrap_or_else(|_| fallback(&parsed.request_id)),
                code,
            )
        }
        Err(message) => {
            let request_id = stable_request_id(args);
            let envelope = unavailable(request_id, ErrorCode::InvalidRequest, &message);
            let code = exit_code(envelope.status);
            (
                render(&envelope).unwrap_or_else(|_| fallback(&envelope.request_id)),
                code,
            )
        }
    }
}

fn success(request_id: String, result: serde_json::Value) -> JsonEnvelope {
    JsonEnvelope {
        schema_version: SCHEMA_VERSION,
        request_id,
        status: OperationStatus::Success,
        result: Some(result),
        error: None,
    }
}

fn unavailable(request_id: String, code: ErrorCode, message: &str) -> JsonEnvelope {
    let message = truncate(message, MAX_ERROR_MESSAGE_BYTES);
    let status = match code {
        ErrorCode::MissingCaptureTool => OperationStatus::MissingTool,
        ErrorCode::SelectionCancelled => OperationStatus::Cancelled,
        ErrorCode::OperationBusy => OperationStatus::Busy,
        ErrorCode::OperationTimeout => OperationStatus::Timeout,
        ErrorCode::RuntimeUnavailable
        | ErrorCode::AdapterUnavailable
        | ErrorCode::AdapterIncompatible => OperationStatus::Unavailable,
        ErrorCode::InvalidRequest | ErrorCode::InvalidEvidence | ErrorCode::NotFound => {
            OperationStatus::Invalid
        }
        _ => OperationStatus::Error,
    };
    JsonEnvelope {
        schema_version: SCHEMA_VERSION,
        request_id,
        status,
        result: None,
        error: Some(JsonError {
            code,
            message,
            retryable: matches!(
                status,
                OperationStatus::Unavailable | OperationStatus::Timeout
            ),
        }),
    }
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn fallback(request_id: &str) -> Vec<u8> {
    let request_id = truncate(request_id, MAX_ID_BYTES);
    format!(
        "{{\"schemaVersion\":1,\"requestId\":\"{request_id}\",\"status\":\"error\",\"result\":null,\"error\":{{\"code\":\"internal\",\"message\":\"response encoding failed\",\"retryable\":false}}}}"
    )
    .into_bytes()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_address(value: &str) -> bool {
    let Some(digits) = value.strip_prefix("0x") else {
        return false;
    };
    !digits.is_empty()
        && value.len() <= MAX_ID_BYTES
        && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn stable_request_id(args: &[String]) -> String {
    let mut digest = Sha256::new();
    for arg in args {
        digest.update(arg.as_bytes());
        digest.update([0]);
    }
    let suffix: String = digest
        .finalize()
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("request-{suffix}")
}

fn stable_capture_id(sha256: &str, region: &surfacecheck_core::Region) -> String {
    let mut digest = Sha256::new();
    digest.update(sha256.as_bytes());
    digest.update(region.x.to_le_bytes());
    digest.update(region.y.to_le_bytes());
    digest.update(region.width.to_le_bytes());
    digest.update(region.height.to_le_bytes());
    let suffix: String = digest
        .finalize()
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("capture-{suffix}")
}

pub fn exit_code(status: OperationStatus) -> i32 {
    match status {
        OperationStatus::Success => 0,
        OperationStatus::Invalid => 2,
        OperationStatus::Unavailable => 3,
        OperationStatus::MissingTool => 4,
        OperationStatus::Cancelled => 5,
        OperationStatus::Busy => 6,
        OperationStatus::Timeout => 7,
        OperationStatus::Error => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parser_requires_one_json_flag_and_rejects_unknown_options() {
        assert!(parse_args(&args(&["status"])).is_err());
        assert!(parse_args(&args(&["status", "--json", "--json"])).is_err());
        assert!(parse_args(&args(&["status", "--json", "--shell"])).is_err());
        assert!(parse_args(&args(&["status", "--json"])).is_ok());
        assert!(parse_args(&args(&["capture", "application", "Firefox", "--json"])).is_err());
    }

    #[test]
    fn parser_accepts_bounded_notes_and_before_after_selection() {
        let parsed = parse_args(&args(&[
            "capture",
            "window",
            "--session",
            "session-1",
            "--note",
            "check spacing",
            "--json",
        ]))
        .expect("capture parse");
        assert!(matches!(parsed.command, Command::CaptureWindow { .. }));
        let selected = parse_args(&args(&[
            "select-before-after",
            "session-1",
            "capture-1",
            "capture-2",
            "--json",
        ]))
        .expect("selection parse");
        assert!(matches!(
            selected.command,
            Command::SelectBeforeAfter { .. }
        ));
    }

    #[test]
    fn consent_and_ids_are_checked_before_runtime_access() {
        let parsed =
            parse_args(&args(&["review", "capture-1", "--agent", "--json"])).expect("parse");
        let response = execute(&parsed);
        assert_eq!(response.status, OperationStatus::Invalid);
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ErrorCode::InvalidRequest)
        );
        assert!(parse_args(&args(&["compare", "same", "same", "--json"])).is_err());
    }

    #[test]
    fn status_is_versioned_and_bounded_without_leaking_paths() {
        let (bytes, code) = run(&args(&["status", "--json"]));
        assert_eq!(code, 0);
        assert!(bytes.len() <= MAX_JSON_FRAME_BYTES);
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("schemaVersion"));
        assert!(!text.contains("/tmp"));
    }
}
