//! Review facts that can be computed without a model or a network.

mod agent;
mod premonition;

use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt;
use surfacecheck_capture::DecodedPng;
use surfacecheck_core::{
    ComparisonRecord, DeterministicCategory, DeterministicFinding, Dimensions, EvidenceRef,
    EvidenceRegion, Scale, Severity, Validate,
};

pub use agent::{
    AgentAdapter, AgentAdapterError, AgentReviewContext, AgentReviewError, CancellationToken,
    CaptureContext, MockAgentAdapter, MockAgentBehavior, ReviewCoordinator,
    AGENT_ADAPTER_PROTOCOL_VERSION,
};
pub use premonition::{
    MockPremonitionAdapter, MockPremonitionBehavior, PremonitionAdapter, PremonitionAdapterError,
    PremonitionCoordinator, PremonitionHandoffError,
};

pub const MAX_REVIEW_PIXELS: u64 = 100_000_000;

#[derive(Debug, Clone)]
pub struct ReviewInput {
    pub capture_id: String,
    pub image: DecodedPng,
    pub scale: Scale,
    pub stale: bool,
    pub duplicate_of: Option<String>,
    pub expected_dimensions: Option<Dimensions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewError {
    InvalidInput,
    IncompatibleDimensions,
    IncompatibleScale,
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "review input is invalid",
            Self::IncompatibleDimensions => "before and after dimensions differ",
            Self::IncompatibleScale => "before and after scales differ",
        })
    }
}

impl Error for ReviewError {}

impl ReviewInput {
    pub fn validate(&self) -> Result<(), ReviewError> {
        if self.capture_id.is_empty()
            || self.capture_id.len() > 128
            || !self
                .capture_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ReviewError::InvalidInput);
        }
        self.image
            .dimensions
            .validate()
            .map_err(|_| ReviewError::InvalidInput)?;
        self.scale
            .validate()
            .map_err(|_| ReviewError::InvalidInput)?;
        let expected_bytes = u64::from(self.image.dimensions.width)
            .checked_mul(u64::from(self.image.dimensions.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ReviewError::InvalidInput)?;
        if expected_bytes > MAX_REVIEW_PIXELS * 4
            || u64::try_from(self.image.pixels.len()).ok() != Some(expected_bytes)
        {
            return Err(ReviewError::InvalidInput);
        }
        if let Some(expected) = &self.expected_dimensions {
            expected.validate().map_err(|_| ReviewError::InvalidInput)?;
        }
        if let Some(duplicate) = &self.duplicate_of {
            if duplicate.is_empty() || duplicate == &self.capture_id {
                return Err(ReviewError::InvalidInput);
            }
        }
        Ok(())
    }

    fn whole_image_evidence(&self) -> EvidenceRef {
        EvidenceRef {
            capture_id: self.capture_id.clone(),
            content_sha256: self.image.sha256.clone(),
            region: EvidenceRegion {
                x: 0,
                y: 0,
                width: self.image.dimensions.width,
                height: self.image.dimensions.height,
            },
        }
    }
}

pub fn deterministic_findings(
    input: &ReviewInput,
) -> Result<Vec<DeterministicFinding>, ReviewError> {
    input.validate()?;
    let evidence = input.whole_image_evidence();
    let mut findings = Vec::new();
    if input.image.pixels.iter().all(|pixel| pixel == &0) {
        findings.push(finding(
            input,
            DeterministicCategory::EmptyCapture,
            Severity::Info,
            "empty_capture",
            "Every captured channel is zero; the image is blank.",
            None,
            &evidence,
        ));
    }
    if input
        .image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel[3] == 0)
    {
        findings.push(finding(
            input,
            DeterministicCategory::EmptyCapture,
            Severity::Info,
            "fully_transparent",
            "Every captured pixel is fully transparent.",
            Some(0.0),
            &evidence,
        ));
    }
    let width =
        usize::try_from(input.image.dimensions.width).map_err(|_| ReviewError::InvalidInput)?;
    let height =
        usize::try_from(input.image.dimensions.height).map_err(|_| ReviewError::InvalidInput)?;
    let mut boundary_pixels = 0u64;
    for y in 0..height {
        for x in 0..width {
            if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
                let offset = (y * width + x) * 4;
                if input.image.pixels[offset..offset + 4] != [0, 0, 0, 0] {
                    boundary_pixels += 1;
                }
            }
        }
    }
    if boundary_pixels > 0 {
        findings.push(finding(
            input,
            DeterministicCategory::BoundaryContact,
            Severity::Info,
            "boundary_contact",
            "Non-transparent pixels touch the capture boundary; this is a measured fact, not a clipping judgement.",
            Some(boundary_pixels as f64),
            &evidence,
        ));
    }
    if input.stale {
        findings.push(finding(
            input,
            DeterministicCategory::StaleCapture,
            Severity::Medium,
            "stale_capture",
            "Window identity or geometry changed while the capture was being made.",
            None,
            &evidence,
        ));
    }
    if let Some(duplicate_of) = &input.duplicate_of {
        findings.push(finding(
            input,
            DeterministicCategory::DuplicateCapture,
            Severity::Info,
            "duplicate_capture",
            &format!("The capture has the same content as {duplicate_of}."),
            None,
            &evidence,
        ));
    }
    if let Some(expected) = &input.expected_dimensions {
        if expected != &input.image.dimensions {
            findings.push(finding(
                input,
                DeterministicCategory::UnexpectedDimensions,
                Severity::Medium,
                "unexpected_dimensions",
                "The capture dimensions differ from the requested dimensions.",
                None,
                &evidence,
            ));
        }
    }
    Ok(findings)
}

pub fn compare(before: &ReviewInput, after: &ReviewInput) -> Result<ComparisonRecord, ReviewError> {
    before.validate()?;
    after.validate()?;
    if before.image.dimensions != after.image.dimensions {
        return Err(ReviewError::IncompatibleDimensions);
    }
    if before.scale != after.scale {
        return Err(ReviewError::IncompatibleScale);
    }
    let mut changed_pixels = 0u64;
    let mut absolute = 0f64;
    let mut squared = 0f64;
    let mut luminance_difference = 0f64;
    let before_pixels = before.image.pixels.as_chunks::<4>().0;
    let after_pixels = after.image.pixels.as_chunks::<4>().0;
    for (left, right) in before_pixels.iter().zip(after_pixels.iter()) {
        if left != right {
            changed_pixels += 1;
        }
        for channel in 0..4 {
            let difference = (f64::from(left[channel]) - f64::from(right[channel])).abs() / 255.0;
            absolute += difference;
            squared += difference * difference;
        }
        let left_luma =
            0.2126 * f64::from(left[0]) + 0.7152 * f64::from(left[1]) + 0.0722 * f64::from(left[2]);
        let right_luma = 0.2126 * f64::from(right[0])
            + 0.7152 * f64::from(right[1])
            + 0.0722 * f64::from(right[2]);
        luminance_difference += (left_luma - right_luma).abs() / 255.0;
    }
    let pixels =
        u64::from(before.image.dimensions.width) * u64::from(before.image.dimensions.height);
    let components = (pixels * 4) as f64;
    let changed_fraction = changed_pixels as f64 / pixels as f64;
    let mean_absolute_difference = absolute / components;
    let rms_difference = (squared / components).sqrt();
    let perceptual_distance = luminance_difference / pixels as f64;
    Ok(ComparisonRecord {
        comparison_id: stable_id(
            "comparison",
            &format!("{}:{}", before.capture_id, after.capture_id),
        ),
        before_capture_id: before.capture_id.clone(),
        after_capture_id: after.capture_id.clone(),
        dimensions: before.image.dimensions.clone(),
        same_scale: true,
        changed_pixels,
        changed_fraction,
        mean_absolute_difference,
        rms_difference,
        perceptual_distance: Some(perceptual_distance),
    })
}

pub fn incompatible_finding(
    before: &ReviewInput,
    after: &ReviewInput,
    error: &ReviewError,
) -> Result<DeterministicFinding, ReviewError> {
    before.validate()?;
    after.validate()?;
    let (category, code, explanation) = match error {
        ReviewError::IncompatibleDimensions => (
            DeterministicCategory::UnexpectedDimensions,
            "comparison_dimensions",
            "Before and after captures have incompatible dimensions.",
        ),
        ReviewError::IncompatibleScale => (
            DeterministicCategory::ScaleInconsistency,
            "comparison_scale",
            "Before and after captures have incompatible fractional scales.",
        ),
        _ => return Err(ReviewError::InvalidInput),
    };
    let evidence = before.whole_image_evidence();
    Ok(finding(
        before,
        category,
        Severity::Info,
        code,
        explanation,
        None,
        &evidence,
    ))
}

fn finding(
    input: &ReviewInput,
    category: DeterministicCategory,
    severity: Severity,
    code: &str,
    explanation: &str,
    measurement: Option<f64>,
    evidence: &EvidenceRef,
) -> DeterministicFinding {
    DeterministicFinding {
        finding_id: stable_id(&input.capture_id, code),
        category,
        severity,
        evidence: vec![evidence.clone()],
        explanation: explanation.to_owned(),
        code: code.to_owned(),
        measurement,
    }
}

fn stable_id(prefix: &str, value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(value.as_bytes());
    let suffix: String = digest
        .finalize()
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{prefix}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use surfacecheck_core::{Dimensions, Scale};

    fn image(id: &str, width: u32, height: u32, pixels: Vec<u8>) -> ReviewInput {
        ReviewInput {
            capture_id: id.into(),
            image: DecodedPng {
                dimensions: Dimensions { width, height },
                pixels,
                has_alpha: true,
                sha256: "0".repeat(64),
            },
            scale: Scale { x: 1.25, y: 1.25 },
            stale: false,
            duplicate_of: None,
            expected_dimensions: None,
        }
    }

    #[test]
    fn identical_images_have_zero_difference() {
        let before = image("before", 2, 1, vec![255, 0, 0, 255, 0, 0, 0, 255]);
        let after = image("after", 2, 1, before.image.pixels.clone());
        let comparison = compare(&before, &after).expect("compatible");
        assert_eq!(comparison.changed_pixels, 0);
        assert_eq!(comparison.changed_fraction, 0.0);
        assert_eq!(comparison.rms_difference, 0.0);
    }

    #[test]
    fn changed_images_have_bounded_metrics_and_stable_findings() {
        let before = image("before", 2, 1, vec![0, 0, 0, 255, 0, 0, 0, 255]);
        let mut after = image("after", 2, 1, vec![0, 0, 0, 255, 0, 0, 0, 255]);
        after.image.pixels[0] = 255;
        let comparison = compare(&before, &after).expect("compatible");
        assert_eq!(comparison.changed_pixels, 1);
        assert!((0.0..=1.0).contains(&comparison.changed_fraction));
        assert!((0.0..=1.0).contains(&comparison.mean_absolute_difference));
        assert!((0.0..=1.0).contains(&comparison.rms_difference));
        let first = deterministic_findings(&before).expect("review");
        let second = deterministic_findings(&before).expect("review");
        assert_eq!(first, second);
    }

    #[test]
    fn transparent_and_boundary_facts_are_informational() {
        let transparent = image("transparent", 2, 2, vec![0; 16]);
        let findings = deterministic_findings(&transparent).expect("review");
        assert!(findings
            .iter()
            .any(|finding| finding.code == "fully_transparent"));
        let edge = image(
            "edge",
            2,
            2,
            vec![255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let findings = deterministic_findings(&edge).expect("review");
        assert!(findings.iter().any(
            |finding| finding.code == "boundary_contact" && finding.severity == Severity::Info
        ));
    }

    #[test]
    fn incompatible_dimensions_and_scales_are_explicit() {
        let before = image("before", 1, 1, vec![0, 0, 0, 255]);
        let after = image("after", 2, 1, vec![0, 0, 0, 255, 0, 0, 0, 255]);
        assert_eq!(
            compare(&before, &after),
            Err(ReviewError::IncompatibleDimensions)
        );
        let finding = incompatible_finding(&before, &after, &ReviewError::IncompatibleDimensions)
            .expect("finding");
        assert_eq!(
            finding.category,
            DeterministicCategory::UnexpectedDimensions
        );
        let mut same = image("same", 1, 1, vec![0, 0, 0, 255]);
        same.scale.x = 2.0;
        assert_eq!(compare(&before, &same), Err(ReviewError::IncompatibleScale));
    }

    #[test]
    fn duplicate_and_stale_flags_are_reported() {
        let mut input = image("capture", 1, 1, vec![0, 0, 0, 255]);
        input.stale = true;
        input.duplicate_of = Some("other".into());
        let findings = deterministic_findings(&input).expect("review");
        assert!(findings
            .iter()
            .any(|finding| finding.code == "stale_capture"));
        assert!(findings
            .iter()
            .any(|finding| finding.code == "duplicate_capture"));
    }
}
