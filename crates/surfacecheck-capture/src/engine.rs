use crate::{
    decode_png, CommandRunner, CommandSpec, DecodedPng, PngLimits, RunnerError, ToolOutput,
};
use serde_json::Value;
use std::fmt;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;
use surfacecheck_core::{CaptureType, ErrorCode, OperationStatus, Region, ToolName, Validate};

const MAX_HYPR_JSON_BYTES: usize = 256 * 1024;
const MAX_SELECTION_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub address: String,
    pub region: Region,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureMode {
    ActiveWindow,
    Region,
    Application { address: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOutcome {
    pub capture_type: CaptureType,
    pub image: DecodedPng,
    pub region: Region,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFailure {
    pub status: OperationStatus,
    pub code: ErrorCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("capture tool output is malformed")
    }
}

impl std::error::Error for ParseError {}

impl fmt::Display for CaptureFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "capture failed: {:?}/{:?}", self.status, self.code)
    }
}

impl std::error::Error for CaptureFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAvailability {
    pub tool: ToolName,
    pub status: OperationStatus,
    pub version: Option<String>,
    pub error: Option<ErrorCode>,
}

pub struct CaptureEngine<R> {
    runner: R,
    pub png_limits: PngLimits,
    pub timeout: Duration,
    cancellation: Option<Arc<AtomicBool>>,
}

impl<R: CommandRunner> CaptureEngine<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            png_limits: PngLimits::default(),
            timeout: Duration::from_secs(30),
            cancellation: None,
        }
    }

    pub fn with_cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn probe_tools(&self) -> Vec<ToolAvailability> {
        [
            (ToolName::Grim, "grim", vec!["--version"]),
            (ToolName::Slurp, "slurp", vec!["--version"]),
            (ToolName::Hyprctl, "hyprctl", vec!["version"]),
        ]
        .into_iter()
        .map(|(tool, program, args)| {
            match self.run(CommandSpec {
                program: program.into(),
                args: args.into_iter().map(String::from).collect(),
                stdin: Vec::new(),
                timeout: self.timeout,
                max_stdout_bytes: 512,
                max_stderr_bytes: 512,
                cancellation: self.cancellation.clone(),
            }) {
                Ok(output) if output.exit_code == Some(0) && !output.output_limited => {
                    ToolAvailability {
                        tool,
                        status: OperationStatus::Success,
                        version: bounded_version(&output.stdout),
                        error: None,
                    }
                }
                Err(CaptureFailure { status, code }) => ToolAvailability {
                    tool,
                    status,
                    version: None,
                    error: Some(code),
                },
                Ok(output) if output.timed_out => ToolAvailability {
                    tool,
                    status: OperationStatus::Timeout,
                    version: None,
                    error: Some(ErrorCode::OperationTimeout),
                },
                Ok(_) => ToolAvailability {
                    tool,
                    status: OperationStatus::Error,
                    version: None,
                    error: Some(ErrorCode::ToolFailed),
                },
            }
        })
        .collect()
    }

    pub fn capture(&self, mode: CaptureMode) -> Result<CaptureOutcome, CaptureFailure> {
        match mode {
            CaptureMode::ActiveWindow => self.capture_window(),
            CaptureMode::Region => self.capture_region(),
            CaptureMode::Application { address } => self.capture_application(&address),
        }
    }

    fn capture_window(&self) -> Result<CaptureOutcome, CaptureFailure> {
        let before = self.active_window()?;
        let image = self.capture_region_with_grim(&before.region)?;
        let after = self.active_window()?;
        Ok(CaptureOutcome {
            capture_type: CaptureType::Window,
            image,
            region: before.region.clone(),
            stale: before != after,
        })
    }

    fn capture_region(&self) -> Result<CaptureOutcome, CaptureFailure> {
        let selection = self.run(CommandSpec {
            program: "slurp".into(),
            args: vec!["-f".into(), "%x,%y %wx%h".into()],
            stdin: Vec::new(),
            timeout: self.timeout,
            max_stdout_bytes: MAX_SELECTION_BYTES,
            max_stderr_bytes: 4096,
            cancellation: self.cancellation.clone(),
        })?;
        if selection.cancelled || (selection.exit_code != Some(0) && selection.stdout.is_empty()) {
            return Err(CaptureFailure {
                status: OperationStatus::Cancelled,
                code: ErrorCode::SelectionCancelled,
            });
        }
        if selection.timed_out {
            return Err(CaptureFailure {
                status: OperationStatus::Timeout,
                code: ErrorCode::OperationTimeout,
            });
        }
        if selection.output_limited {
            return Err(CaptureFailure {
                status: OperationStatus::Invalid,
                code: ErrorCode::InvalidRequest,
            });
        }
        let region = parse_selection(&selection.stdout).map_err(|_| CaptureFailure {
            status: OperationStatus::Invalid,
            code: ErrorCode::InvalidRequest,
        })?;
        let image = self.capture_region_with_grim(&region)?;
        Ok(CaptureOutcome {
            capture_type: CaptureType::Region,
            image,
            region,
            stale: false,
        })
    }

    fn capture_application(&self, address: &str) -> Result<CaptureOutcome, CaptureFailure> {
        validate_address(address).map_err(|_| CaptureFailure {
            status: OperationStatus::Invalid,
            code: ErrorCode::InvalidRequest,
        })?;
        let before = self
            .clients()?
            .into_iter()
            .find(|window| window.address == address)
            .ok_or(CaptureFailure {
                status: OperationStatus::Invalid,
                code: ErrorCode::NotFound,
            })?;
        let image = self.capture_region_with_grim(&before.region)?;
        let after = self
            .clients()?
            .into_iter()
            .find(|window| window.address == address);
        Ok(CaptureOutcome {
            capture_type: CaptureType::Application,
            image,
            region: before.region.clone(),
            stale: after.as_ref() != Some(&before),
        })
    }

    fn active_window(&self) -> Result<WindowSnapshot, CaptureFailure> {
        let output = self.run_json_command(vec!["-j", "activewindow"])?;
        parse_window(&output.stdout).map_err(|_| CaptureFailure {
            status: OperationStatus::Invalid,
            code: ErrorCode::InvalidEvidence,
        })
    }

    fn clients(&self) -> Result<Vec<WindowSnapshot>, CaptureFailure> {
        let output = self.run_json_command(vec!["-j", "clients"])?;
        parse_clients(&output.stdout).map_err(|_| CaptureFailure {
            status: OperationStatus::Invalid,
            code: ErrorCode::InvalidEvidence,
        })
    }

    fn run_json_command(&self, args: Vec<&str>) -> Result<ToolOutput, CaptureFailure> {
        let output = self.run(CommandSpec {
            program: "hyprctl".into(),
            args: args.into_iter().map(String::from).collect(),
            stdin: Vec::new(),
            timeout: self.timeout,
            max_stdout_bytes: MAX_HYPR_JSON_BYTES,
            max_stderr_bytes: 4096,
            cancellation: self.cancellation.clone(),
        })?;
        if output.timed_out {
            return Err(CaptureFailure {
                status: OperationStatus::Timeout,
                code: ErrorCode::OperationTimeout,
            });
        }
        if output.cancelled {
            return Err(CaptureFailure {
                status: OperationStatus::Cancelled,
                code: ErrorCode::SelectionCancelled,
            });
        }
        if output.output_limited {
            return Err(CaptureFailure {
                status: OperationStatus::Invalid,
                code: ErrorCode::InvalidEvidence,
            });
        }
        if output.exit_code != Some(0) {
            return Err(CaptureFailure {
                status: OperationStatus::Error,
                code: ErrorCode::ToolFailed,
            });
        }
        Ok(output)
    }

    fn capture_region_with_grim(&self, region: &Region) -> Result<DecodedPng, CaptureFailure> {
        let output = self.run(CommandSpec {
            program: "grim".into(),
            args: vec!["-g".into(), format_region(region)],
            stdin: Vec::new(),
            timeout: self.timeout,
            max_stdout_bytes: self.png_limits.max_file_bytes,
            max_stderr_bytes: 4096,
            cancellation: self.cancellation.clone(),
        })?;
        if output.timed_out {
            return Err(CaptureFailure {
                status: OperationStatus::Timeout,
                code: ErrorCode::OperationTimeout,
            });
        }
        if output.cancelled {
            return Err(CaptureFailure {
                status: OperationStatus::Cancelled,
                code: ErrorCode::SelectionCancelled,
            });
        }
        if output.output_limited {
            return Err(CaptureFailure {
                status: OperationStatus::Invalid,
                code: ErrorCode::InvalidEvidence,
            });
        }
        if output.exit_code != Some(0) {
            return Err(CaptureFailure {
                status: OperationStatus::Error,
                code: ErrorCode::ToolFailed,
            });
        }
        decode_png(&output.stdout, self.png_limits).map_err(|_| CaptureFailure {
            status: OperationStatus::Invalid,
            code: ErrorCode::InvalidEvidence,
        })
    }

    fn run(&self, spec: CommandSpec) -> Result<ToolOutput, CaptureFailure> {
        self.runner.run(&spec).map_err(|error| match error {
            RunnerError::Missing => CaptureFailure {
                status: OperationStatus::MissingTool,
                code: ErrorCode::MissingCaptureTool,
            },
            RunnerError::Spawn | RunnerError::Io => CaptureFailure {
                status: OperationStatus::Error,
                code: ErrorCode::ToolFailed,
            },
        })
    }
}

fn bounded_version(output: &[u8]) -> Option<String> {
    let line = output
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .next()?;
    let value = std::str::from_utf8(line).ok()?.trim();
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        None
    } else {
        Some(value.to_owned())
    }
}

pub fn parse_selection(input: &[u8]) -> Result<Region, ParseError> {
    let text = std::str::from_utf8(input).map_err(|_| ParseError)?.trim();
    let mut fields = text.split_whitespace();
    let position = fields.next().ok_or(ParseError)?;
    let dimensions = fields.next().ok_or(ParseError)?;
    if fields.next().is_some() {
        return Err(ParseError);
    }
    let mut position = position.split(',');
    let x = position
        .next()
        .ok_or(ParseError)?
        .parse::<i64>()
        .map_err(|_| ParseError)?;
    let y = position
        .next()
        .ok_or(ParseError)?
        .parse::<i64>()
        .map_err(|_| ParseError)?;
    if position.next().is_some() {
        return Err(ParseError);
    }
    let mut dimensions = dimensions.split('x');
    let width = dimensions
        .next()
        .ok_or(ParseError)?
        .parse::<u32>()
        .map_err(|_| ParseError)?;
    let height = dimensions
        .next()
        .ok_or(ParseError)?
        .parse::<u32>()
        .map_err(|_| ParseError)?;
    if dimensions.next().is_some() {
        return Err(ParseError);
    }
    let region = Region {
        x,
        y,
        width,
        height,
    };
    region.validate().map_err(|_| ParseError)?;
    Ok(region)
}

pub fn parse_clients(input: &[u8]) -> Result<Vec<WindowSnapshot>, ParseError> {
    let value: Value = serde_json::from_slice(input).map_err(|_| ParseError)?;
    let windows = value.as_array().ok_or(ParseError)?;
    if windows.len() > 256 {
        return Err(ParseError);
    }
    windows
        .iter()
        .map(|window| parse_window_value(window).map_err(|_| ParseError))
        .collect()
}

fn parse_window(input: &[u8]) -> Result<WindowSnapshot, ()> {
    let value: Value = serde_json::from_slice(input).map_err(|_| ())?;
    parse_window_value(&value)
}

fn parse_window_value(value: &Value) -> Result<WindowSnapshot, ()> {
    let object = value.as_object().ok_or(())?;
    let address = object
        .get("address")
        .and_then(Value::as_str)
        .ok_or(())?
        .to_owned();
    validate_address(&address)?;
    let at = parse_pair_i64(object.get("at"))?;
    let size = parse_pair_u32(object.get("size"))?;
    let region = Region {
        x: at[0],
        y: at[1],
        width: size[0],
        height: size[1],
    };
    region.validate().map_err(|_| ())?;
    // Deliberately do not read or retain the hostile title/class/initialClass fields.
    Ok(WindowSnapshot { address, region })
}

fn parse_pair_i64(value: Option<&Value>) -> Result<[i64; 2], ()> {
    let array = value.and_then(Value::as_array).ok_or(())?;
    if array.len() != 2 {
        return Err(());
    }
    Ok([array[0].as_i64().ok_or(())?, array[1].as_i64().ok_or(())?])
}

fn parse_pair_u32(value: Option<&Value>) -> Result<[u32; 2], ()> {
    let array = value.and_then(Value::as_array).ok_or(())?;
    if array.len() != 2 {
        return Err(());
    }
    Ok([
        array[0]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(())?,
        array[1]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(())?,
    ])
}

fn validate_address(address: &str) -> Result<(), ()> {
    if address.is_empty()
        || address.len() > 64
        || !address
            .strip_prefix("0x")
            .unwrap_or(address)
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(());
    }
    Ok(())
}

fn format_region(region: &Region) -> String {
    format!(
        "{},{} {}x{}",
        region.x, region.y, region.width, region.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::png::test_png;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<Result<ToolOutput, RunnerError>>>,
        specs: Mutex<Vec<CommandSpec>>,
    }

    impl FakeRunner {
        fn new(outputs: Vec<Result<ToolOutput, RunnerError>>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                specs: Mutex::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, spec: &CommandSpec) -> Result<ToolOutput, RunnerError> {
            self.specs.lock().expect("spec lock").push(spec.clone());
            self.outputs
                .lock()
                .expect("output lock")
                .pop_front()
                .expect("fake output")
        }
    }

    fn output(stdout: Vec<u8>) -> Result<ToolOutput, RunnerError> {
        Ok(ToolOutput {
            exit_code: Some(0),
            stdout,
            stderr: Vec::new(),
            timed_out: false,
            cancelled: false,
            output_limited: false,
        })
    }

    #[test]
    fn parses_negative_selection_and_ignores_hostile_titles() {
        assert_eq!(
            parse_selection(b"-1920,4 800x600").expect("selection").x,
            -1920
        );
        let json =
            br#"{"address":"0xabc","at":[-1920,4],"size":[800,600],"title":"secret\u0000url"}"#;
        assert_eq!(parse_window(json).expect("window").region.x, -1920);
    }

    #[test]
    fn region_cancel_is_distinct_from_tool_failure() {
        let runner = FakeRunner::new(vec![output(Vec::new()).map(|mut value| {
            value.exit_code = Some(1);
            value
        })]);
        let engine = CaptureEngine::new(runner);
        let error = engine.capture(CaptureMode::Region).expect_err("cancel");
        assert_eq!(error.status, OperationStatus::Cancelled);
        assert_eq!(error.code, ErrorCode::SelectionCancelled);
    }

    #[test]
    fn active_window_focus_change_marks_capture_stale() {
        let first = br#"{"address":"0xabc","at":[0,0],"size":[1,1],"title":"first"}"#.to_vec();
        let second = br#"{"address":"0xdef","at":[0,0],"size":[1,1],"title":"second"}"#.to_vec();
        let runner = FakeRunner::new(vec![output(first), output(test_png(1, 1)), output(second)]);
        let engine = CaptureEngine::new(runner);
        let outcome = engine.capture(CaptureMode::ActiveWindow).expect("capture");
        assert!(outcome.stale);
    }

    #[test]
    fn app_capture_requires_exact_address_and_uses_fixed_argv() {
        let clients =
            br#"[{"address":"0xabc","at":[-2,3],"size":[1,1],"title":"secret"}]"#.to_vec();
        let runner = FakeRunner::new(vec![
            output(clients),
            output(test_png(1, 1)),
            output(br#"[{"address":"0xabc","at":[-2,3],"size":[1,1]}]"#.to_vec()),
        ]);
        let engine = CaptureEngine::new(runner);
        let outcome = engine
            .capture(CaptureMode::Application {
                address: "0xabc".into(),
            })
            .expect("capture");
        assert!(!outcome.stale);
    }

    #[test]
    fn missing_tool_is_honest() {
        let runner = FakeRunner::new(vec![Err(RunnerError::Missing)]);
        let engine = CaptureEngine::new(runner);
        let error = engine
            .capture(CaptureMode::Region)
            .expect_err("missing slurp");
        assert_eq!(error.status, OperationStatus::MissingTool);
    }

    #[test]
    fn timeout_and_crash_are_not_reported_as_invalid_images() {
        let timeout = ToolOutput {
            exit_code: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            timed_out: true,
            cancelled: false,
            output_limited: false,
        };
        let runner = FakeRunner::new(vec![Ok(timeout)]);
        let error = CaptureEngine::new(runner)
            .capture(CaptureMode::ActiveWindow)
            .expect_err("timeout");
        assert_eq!(error.status, OperationStatus::Timeout);

        let failed = ToolOutput {
            exit_code: Some(127),
            stdout: Vec::new(),
            stderr: b"secret command details".to_vec(),
            timed_out: false,
            cancelled: false,
            output_limited: false,
        };
        let runner = FakeRunner::new(vec![Ok(failed)]);
        let error = CaptureEngine::new(runner)
            .capture(CaptureMode::ActiveWindow)
            .expect_err("crash");
        assert_eq!(error.status, OperationStatus::Error);
        assert_eq!(error.code, ErrorCode::ToolFailed);
    }
}
