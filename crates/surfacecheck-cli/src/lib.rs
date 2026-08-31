//! Strict, bounded command parsing and JSON responses.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use surfacecheck_capture::{CaptureEngine, CaptureMode, DirectCommandRunner};
use surfacecheck_core::{
    ErrorCode, OperationStatus, MAX_ERROR_MESSAGE_BYTES, MAX_JSON_FRAME_BYTES, SCHEMA_VERSION,
};
use surfacecheck_service::RuntimePaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Status,
    CaptureWindow,
    CaptureRegion,
    CaptureApplication {
        address: String,
    },
    Review {
        capture_id: String,
        agent: bool,
        consent: bool,
    },
    Compare {
        before_id: String,
        after_id: String,
    },
    Export {
        session_id: String,
    },
    HandoffPremonition {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonEnvelope {
    pub schema_version: u16,
    pub request_id: String,
    pub status: OperationStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonError>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    runtime_configured: bool,
    service_socket_present: bool,
    tools: Vec<ToolStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureResult {
    capture_id: String,
    capture_type: surfacecheck_core::CaptureType,
    width: u32,
    height: u32,
    sha256: String,
    stale: bool,
    stored: bool,
}

pub fn parse_args(args: &[String]) -> Result<ParsedCommand, String> {
    if args.is_empty() {
        return Err("a command is required".into());
    }
    let mut positional = Vec::new();
    let mut json_count = 0usize;
    let mut agent = false;
    let mut consent = false;
    for arg in args {
        if arg == "--json" {
            json_count += 1;
        } else if arg == "--agent" {
            agent = true;
        } else if arg == "--consent-local" || arg == "--consent-external" {
            consent = true;
        } else if arg.starts_with('-') {
            return Err("unknown option".into());
        } else {
            positional.push(arg.as_str());
        }
    }
    if json_count != 1 {
        return Err("exactly one --json option is required".into());
    }
    let command = match positional.as_slice() {
        ["status"] => Command::Status,
        ["capture", "window"] => Command::CaptureWindow,
        ["capture", "region"] => Command::CaptureRegion,
        ["capture", "application", address] if valid_id_or_address(address) => {
            Command::CaptureApplication {
                address: (*address).to_owned(),
            }
        }
        ["review", capture_id] if valid_id(capture_id) => Command::Review {
            capture_id: (*capture_id).to_owned(),
            agent,
            consent,
        },
        ["compare", before_id, after_id] if valid_id(before_id) && valid_id(after_id) => {
            if before_id == after_id {
                return Err("before and after capture IDs must differ".into());
            }
            Command::Compare {
                before_id: (*before_id).to_owned(),
                after_id: (*after_id).to_owned(),
            }
        }
        ["export", session_id] if valid_id(session_id) => Command::Export {
            session_id: (*session_id).to_owned(),
        },
        ["handoff", "premonition", finding_id] if valid_id(finding_id) => {
            Command::HandoffPremonition {
                finding_id: (*finding_id).to_owned(),
                consent,
            }
        }
        ["cancel", operation_id, generation] if valid_id(operation_id) => {
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

pub fn execute(parsed: &ParsedCommand) -> JsonEnvelope {
    let request_id = parsed.request_id.clone();
    match &parsed.command {
        Command::Status => status(request_id),
        Command::CaptureWindow => capture(request_id, CaptureMode::ActiveWindow),
        Command::CaptureRegion => capture(request_id, CaptureMode::Region),
        Command::CaptureApplication { address } => capture(
            request_id,
            CaptureMode::Application {
                address: address.clone(),
            },
        ),
        Command::Review {
            agent,
            consent,
            capture_id: _,
        } => {
            if !agent {
                unavailable(
                    request_id,
                    ErrorCode::InvalidRequest,
                    "explicit --agent is required",
                )
            } else if !consent {
                unavailable(
                    request_id,
                    ErrorCode::InvalidRequest,
                    "explicit --consent-local is required",
                )
            } else {
                unavailable(
                    request_id,
                    ErrorCode::RuntimeUnavailable,
                    "review service is not connected",
                )
            }
        }
        Command::Compare { .. } => unavailable(
            request_id,
            ErrorCode::RuntimeUnavailable,
            "comparison service is not connected",
        ),
        Command::Export { .. } => unavailable(
            request_id,
            ErrorCode::RuntimeUnavailable,
            "export service is not connected",
        ),
        Command::HandoffPremonition { consent, .. } => {
            if !consent {
                unavailable(
                    request_id,
                    ErrorCode::InvalidRequest,
                    "explicit --consent-external is required",
                )
            } else {
                unavailable(
                    request_id,
                    ErrorCode::AdapterUnavailable,
                    "no Premonition adapter contract is available",
                )
            }
        }
        Command::Cancel { .. } => unavailable(
            request_id,
            ErrorCode::RuntimeUnavailable,
            "runtime service is not connected",
        ),
    }
}

pub fn render(envelope: &JsonEnvelope) -> Result<Vec<u8>, String> {
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
            (
                render(&envelope).unwrap_or_else(|_| fallback(&envelope.request_id)),
                exit_code(envelope.status),
            )
        }
    }
}

fn status(request_id: String) -> JsonEnvelope {
    let runtime_configured = RuntimePaths::from_environment().is_ok();
    let service_socket_present = RuntimePaths::from_environment()
        .map(|paths| paths.socket.exists())
        .unwrap_or(false);
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
            runtime_configured,
            service_socket_present,
            tools,
        })
        .unwrap_or_else(|_| serde_json::json!({})),
    )
}

fn capture(request_id: String, mode: CaptureMode) -> JsonEnvelope {
    let engine = CaptureEngine::new(DirectCommandRunner);
    match engine.capture(mode) {
        Ok(outcome) => success(
            request_id,
            serde_json::to_value(CaptureResult {
                capture_id: stable_capture_id(&outcome.image.sha256, &outcome.region),
                capture_type: outcome.capture_type,
                width: outcome.image.dimensions.width,
                height: outcome.image.dimensions.height,
                sha256: outcome.image.sha256,
                stale: outcome.stale,
                stored: false,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        ),
        Err(error) => unavailable(request_id, error.code, capture_error_message(error.status)),
    }
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
    let message = if message.len() > MAX_ERROR_MESSAGE_BYTES {
        &message[..MAX_ERROR_MESSAGE_BYTES]
    } else {
        message
    };
    let status = match code {
        ErrorCode::MissingCaptureTool => OperationStatus::MissingTool,
        ErrorCode::SelectionCancelled => OperationStatus::Cancelled,
        ErrorCode::OperationBusy => OperationStatus::Busy,
        ErrorCode::OperationTimeout => OperationStatus::Timeout,
        ErrorCode::RuntimeUnavailable | ErrorCode::AdapterUnavailable => {
            OperationStatus::Unavailable
        }
        ErrorCode::InvalidRequest | ErrorCode::InvalidEvidence => OperationStatus::Invalid,
        _ => OperationStatus::Error,
    };
    JsonEnvelope {
        schema_version: SCHEMA_VERSION,
        request_id,
        status,
        result: None,
        error: Some(JsonError {
            code,
            message: message.to_owned(),
            retryable: matches!(
                status,
                OperationStatus::Unavailable | OperationStatus::Timeout
            ),
        }),
    }
}

fn fallback(request_id: &str) -> Vec<u8> {
    format!(
        "{{\"schemaVersion\":1,\"requestId\":\"{request_id}\",\"status\":\"error\",\"result\":null,\"error\":{{\"code\":\"internal\",\"message\":\"response encoding failed\",\"retryable\":false}}}}"
    )
    .into_bytes()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_id_or_address(value: &str) -> bool {
    valid_id(value)
        || (value.len() <= 64
            && value
                .strip_prefix("0x")
                .unwrap_or(value)
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()))
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
