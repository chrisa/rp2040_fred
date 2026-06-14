use std::io;

use rp2040_fred_protocol::bridge_proto::CommandBlockRequest;

use crate::motion::AxisCalibration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CannedCycleCode {
    G80,
    G81,
    G82,
    G83,
    G84,
}

impl CannedCycleCode {
    pub fn parse(value: &str) -> io::Result<Self> {
        let value = value.trim().to_ascii_uppercase();
        match value.as_str() {
            "80" | "G80" => Ok(Self::G80),
            "81" | "G81" => Ok(Self::G81),
            "82" | "G82" => Ok(Self::G82),
            "83" | "G83" => Ok(Self::G83),
            "84" | "G84" => Ok(Self::G84),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported canned cycle code: {value}"),
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::G80 => "G80",
            Self::G81 => "G81",
            Self::G82 => "G82",
            Self::G83 => "G83",
            Self::G84 => "G84",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CannedCycleParams {
    pub x_mm: Option<f32>,
    pub z_mm: Option<f32>,
    pub i: Option<f32>,
    pub k: Option<f32>,
    pub f: Option<f32>,
    pub slew: u16,
}

pub fn canned_cycle_command_requests_mm(
    code: CannedCycleCode,
    params: CannedCycleParams,
    _calibration: AxisCalibration,
) -> io::Result<Vec<CommandBlockRequest>> {
    match code {
        CannedCycleCode::G80 => Ok(Vec::new()),
        CannedCycleCode::G81 => {
            let _ = required_finite(params.x_mm, "X")?;
            let _ = required_finite(params.z_mm, "Z")?;
            let cuts = required_finite(params.i, "I")?;
            validate_cut_count("I", cuts)?;
            validate_positive("F", required_finite(params.f, "F")?)?;
            unsupported_cycle_mapping(code)
        }
        CannedCycleCode::G82 => {
            let _ = required_finite(params.x_mm, "X")?;
            let _ = required_finite(params.z_mm, "Z")?;
            let cuts = required_finite(params.i, "I")?;
            validate_cut_count("I", cuts)?;
            validate_positive("F", required_finite(params.f, "F")?)?;
            unsupported_cycle_mapping(code)
        }
        CannedCycleCode::G83 => {
            let _ = required_finite(params.z_mm, "Z")?;
            validate_positive("I", required_finite(params.i, "I")?)?;
            let reduction = required_finite(params.k, "K")?;
            if !(reduction > 0.0 && reduction <= 1.0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "K reduction factor must be in range 0.0 < K <= 1.0",
                ));
            }
            validate_positive("F", required_finite(params.f, "F")?)?;
            unsupported_cycle_mapping(code)
        }
        CannedCycleCode::G84 => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "G84 is host-expanded threading; use thread_sync_move/G33",
        )),
    }
}

fn unsupported_cycle_mapping(code: CannedCycleCode) -> io::Result<Vec<CommandBlockRequest>> {
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "{} canned-cycle command-block mapping is not decoded yet",
            code.label()
        ),
    ))
}

fn required_finite(value: Option<f32>, name: &str) -> io::Result<f32> {
    let value = value.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing required {name} word"),
        )
    })?;
    if !value.is_finite() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} word must be finite"),
        ));
    }
    Ok(value)
}

fn validate_positive(name: &str, value: f32) -> io::Result<()> {
    if value <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} word must be greater than zero"),
        ));
    }
    Ok(())
}

fn validate_cut_count(name: &str, value: f32) -> io::Result<()> {
    validate_positive(name, value)?;
    if (value.round() - value).abs() > f32::EPSILON {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} cut count must be an integer"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{canned_cycle_command_requests_mm, CannedCycleCode, CannedCycleParams};
    use crate::motion::AxisCalibration;

    fn cal() -> AxisCalibration {
        AxisCalibration {
            x_counts_per_mm: 100.0,
            z_counts_per_mm: 100.0,
        }
    }

    #[test]
    fn g80_is_noop_cancel() {
        let requests = canned_cycle_command_requests_mm(
            CannedCycleCode::G80,
            CannedCycleParams::default(),
            cal(),
        )
        .expect("g80");

        assert!(requests.is_empty());
    }

    #[test]
    fn g84_is_rejected_with_g33_message() {
        let err = canned_cycle_command_requests_mm(
            CannedCycleCode::G84,
            CannedCycleParams {
                z_mm: Some(10.0),
                i: Some(1.0),
                k: Some(4.0),
                f: Some(1.5),
                slew: 61,
                ..CannedCycleParams::default()
            },
            cal(),
        )
        .expect_err("g84");

        assert!(err.to_string().contains("thread_sync_move/G33"));
    }

    #[test]
    fn g81_validates_required_words_before_mapping_error() {
        let err = canned_cycle_command_requests_mm(
            CannedCycleCode::G81,
            CannedCycleParams {
                z_mm: Some(10.0),
                i: Some(4.0),
                f: Some(100.0),
                slew: 61,
                ..CannedCycleParams::default()
            },
            cal(),
        )
        .expect_err("missing x");

        assert!(err.to_string().contains("missing required X word"));
    }

    #[test]
    fn g83_validates_reduction_factor() {
        let err = canned_cycle_command_requests_mm(
            CannedCycleCode::G83,
            CannedCycleParams {
                z_mm: Some(10.0),
                i: Some(6.0),
                k: Some(1.2),
                f: Some(80.0),
                slew: 61,
                ..CannedCycleParams::default()
            },
            cal(),
        )
        .expect_err("bad k");

        assert!(err.to_string().contains("K reduction factor"));
    }

    #[test]
    fn g81_valid_params_reach_explicit_undecoded_error() {
        let err = canned_cycle_command_requests_mm(
            CannedCycleCode::G81,
            CannedCycleParams {
                x_mm: Some(1.0),
                z_mm: Some(10.0),
                i: Some(4.0),
                f: Some(100.0),
                slew: 61,
                ..CannedCycleParams::default()
            },
            cal(),
        )
        .expect_err("undecoded");

        assert!(err.to_string().contains("not decoded yet"));
    }
}
