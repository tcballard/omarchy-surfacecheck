use serde::{Deserialize, Serialize};
use surfacecheck_core::{Dimensions, Scale, Validate, ValidationError, MAX_IMAGE_DIMENSION};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonitorGeometry {
    pub id: String,
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
    pub scale: Scale,
    pub transform: Transform,
}

impl Validate for MonitorGeometry {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(ValidationError::new(
                "monitor.id",
                "must be a bounded opaque identifier",
            ));
        }
        if self.width == 0
            || self.height == 0
            || self.width > MAX_IMAGE_DIMENSION
            || self.height > MAX_IMAGE_DIMENSION
        {
            return Err(ValidationError::new(
                "monitor",
                "dimensions are outside the supported bounds",
            ));
        }
        self.scale.validate()
    }
}

impl MonitorGeometry {
    pub fn dimensions_after_transform(&self) -> Dimensions {
        match self.transform {
            Transform::Rotate90
            | Transform::Rotate270
            | Transform::Flipped90
            | Transform::Flipped270 => Dimensions {
                width: self.height,
                height: self.width,
            },
            _ => Dimensions {
                width: self.width,
                height: self.height,
            },
        }
    }

    pub fn right(&self) -> Option<i64> {
        self.x.checked_add(i64::from(self.width))
    }

    pub fn bottom(&self) -> Option<i64> {
        self.y.checked_add(i64::from(self.height))
    }
}

pub fn validate_monitors(monitors: &[MonitorGeometry]) -> Result<(), ValidationError> {
    if monitors.is_empty() {
        return Err(ValidationError::new(
            "monitors",
            "at least one monitor is required",
        ));
    }
    if monitors.len() > 32 {
        return Err(ValidationError::new("monitors", "too many monitors"));
    }
    for monitor in monitors {
        monitor.validate()?;
        monitor
            .right()
            .ok_or_else(|| ValidationError::new("monitor.x", "coordinate overflows"))?;
        monitor
            .bottom()
            .ok_or_else(|| ValidationError::new("monitor.y", "coordinate overflows"))?;
    }
    for (index, monitor) in monitors.iter().enumerate() {
        if monitors[..index]
            .iter()
            .any(|previous| previous.id == monitor.id)
        {
            return Err(ValidationError::new(
                "monitors",
                "monitor IDs must be unique",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: &str, x: i64, y: i64, transform: Transform) -> MonitorGeometry {
        MonitorGeometry {
            id: id.to_owned(),
            x,
            y,
            width: 1_920,
            height: 1_080,
            scale: Scale { x: 1.25, y: 1.25 },
            transform,
        }
    }

    #[test]
    fn accepts_negative_origins_and_rotated_dimensions() {
        let rotated = monitor("left", -1_920, 0, Transform::Rotate90);
        assert_eq!(
            rotated.dimensions_after_transform(),
            Dimensions {
                width: 1_080,
                height: 1_920
            }
        );
        assert!(validate_monitors(&[rotated, monitor("main", 0, 0, Transform::Normal)]).is_ok());
    }

    #[test]
    fn rejects_duplicate_ids_and_overflowing_coordinates() {
        assert!(validate_monitors(&[
            monitor("main", 0, 0, Transform::Normal),
            monitor("main", 2_000, 0, Transform::Normal)
        ])
        .is_err());
        assert!(validate_monitors(&[monitor("edge", i64::MAX, 0, Transform::Normal)]).is_err());
    }

    #[test]
    fn rejects_non_finite_fractional_scale() {
        let mut value = monitor("main", 0, 0, Transform::Normal);
        value.scale.x = f64::NAN;
        assert!(value.validate().is_err());
    }
}
