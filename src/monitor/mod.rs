use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(windows)]
mod lg_intel;
#[cfg(windows)]
mod windows;

pub const INPUT_SOURCE_VCP_CODE: u8 = 0x60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputPreset {
    pub value: u32,
    pub label: String,
    #[serde(default)]
    pub favorite: bool,
}

impl InputPreset {
    pub fn new(value: u32, label: impl Into<String>, favorite: bool) -> Self {
        Self {
            value,
            label: label.into(),
            favorite,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub id: String,
    pub device_name: String,
    pub description: String,
    pub model: Option<String>,
    pub supported_inputs: Option<Vec<u32>>,
    pub current_input: Option<u32>,
    pub maximum_input: Option<u32>,
    pub ddc_supported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchOutcome {
    Verified,
    CommandAccepted,
    AlternateCommandSent,
}

#[derive(Debug, Error)]
pub enum MonitorError {
    #[cfg(windows)]
    #[error("Windows monitor API failed while {operation}: {message}")]
    WindowsApi {
        operation: &'static str,
        message: String,
    },
    #[cfg(windows)]
    #[error("Monitor '{0}' is no longer available")]
    NotFound(String),
    #[cfg(windows)]
    #[error(
        "Monitor accepted input 0x{requested:02X}, but still reports 0x{reported:02X}; verify the port's VCP value and disable Auto Input in the monitor OSD"
    )]
    InputNotApplied { requested: u32, reported: u32 },
    #[cfg(windows)]
    #[error("LG alternate input switching failed: {0}")]
    AlternateInput(String),
    #[cfg(not(windows))]
    #[error("This platform is not supported; LG Monitor Control currently requires Windows")]
    UnsupportedPlatform,
}

pub fn default_inputs() -> Vec<InputPreset> {
    vec![
        InputPreset::new(0x0f, "DisplayPort 1", true),
        InputPreset::new(0x10, "DisplayPort 2 / USB-C", false),
        InputPreset::new(0x11, "HDMI 1", true),
        InputPreset::new(0x12, "HDMI 2", true),
        InputPreset::new(0x13, "HDMI 3", false),
        InputPreset::new(0x1b, "USB-C", true),
    ]
}

pub fn input_name(value: u32) -> String {
    match value {
        0x01 => "VGA 1".into(),
        0x02 => "VGA 2".into(),
        0x03 => "DVI 1".into(),
        0x04 => "DVI 2".into(),
        0x0f => "DisplayPort 1".into(),
        0x10 => "DisplayPort 2 / USB-C".into(),
        0x11 => "HDMI 1".into(),
        0x12 => "HDMI 2".into(),
        0x13 => "HDMI 3".into(),
        0x14 => "HDMI 4".into(),
        0x1b => "USB-C".into(),
        _ => format!("Input 0x{value:02X}"),
    }
}

pub(crate) fn model_from_capabilities(capabilities: &str) -> Option<String> {
    capability_group(capabilities, "model")
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

pub(crate) fn inputs_from_capabilities(capabilities: &str) -> Option<Vec<u32>> {
    let vcp = capability_group(capabilities, "vcp")?;
    let bytes = vcp.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        let token_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_hexdigit() {
            cursor += 1;
        }
        if token_start == cursor {
            cursor += 1;
            continue;
        }

        let token = &vcp[token_start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }

        if cursor < bytes.len() && bytes[cursor] == b'(' {
            let (values, next_cursor) = parenthesized_content(vcp, cursor)?;
            if token.eq_ignore_ascii_case("60") {
                let mut inputs: Vec<u32> = values
                    .split_ascii_whitespace()
                    .filter_map(|value| u32::from_str_radix(value, 16).ok())
                    .collect();
                inputs.sort_unstable();
                inputs.dedup();
                return (!inputs.is_empty()).then_some(inputs);
            }
            cursor = next_cursor;
        }
    }

    None
}

fn capability_group<'a>(capabilities: &'a str, name: &str) -> Option<&'a str> {
    let lower = capabilities.to_ascii_lowercase();
    let start = lower.find(&name.to_ascii_lowercase())? + name.len();
    let bytes = capabilities.as_bytes();
    let mut open = start;
    while open < bytes.len() && bytes[open].is_ascii_whitespace() {
        open += 1;
    }
    (bytes.get(open) == Some(&b'('))
        .then(|| parenthesized_content(capabilities, open))
        .flatten()
        .map(|(content, _)| content)
}

fn parenthesized_content(value: &str, open: usize) -> Option<(&str, usize)> {
    let bytes = value.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }

    let mut depth = 0_u32;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((&value[open + 1..index], index + 1));
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(windows)]
pub fn run_helper_command() -> Option<i32> {
    lg_intel::run_helper_command()
}

#[cfg(not(windows))]
pub fn run_helper_command() -> Option<i32> {
    None
}

#[cfg(windows)]
pub fn scan_monitors() -> Result<Vec<MonitorInfo>, MonitorError> {
    windows::scan_monitors()
}

#[cfg(not(windows))]
pub fn scan_monitors() -> Result<Vec<MonitorInfo>, MonitorError> {
    Err(MonitorError::UnsupportedPlatform)
}

#[cfg(windows)]
pub fn set_input(monitor_id: &str, input: u32) -> Result<SwitchOutcome, MonitorError> {
    windows::set_input(monitor_id, input)
}

#[cfg(not(windows))]
pub fn set_input(_monitor_id: &str, _input: u32) -> Result<SwitchOutcome, MonitorError> {
    Err(MonitorError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_and_custom_input_names() {
        assert_eq!(input_name(0x11), "HDMI 1");
        assert_eq!(input_name(0xaa), "Input 0xAA");
    }

    #[test]
    fn parses_model_and_input_values_from_capabilities() {
        let capabilities = "(prot(monitor)type(lcd)model(WK95U)cmds(01 02)vcp(10 14(05 08 0B) 60(11 12 0F 10 ) AC)mccs_ver(2.1))";

        assert_eq!(
            model_from_capabilities(capabilities).as_deref(),
            Some("WK95U")
        );
        assert_eq!(
            inputs_from_capabilities(capabilities),
            Some(vec![0x0f, 0x10, 0x11, 0x12])
        );
    }

    #[test]
    fn returns_none_when_input_values_are_not_advertised() {
        assert_eq!(inputs_from_capabilities("(vcp(10 12 60))"), None);
    }

    #[test]
    fn default_inputs_have_unique_values() {
        let inputs = default_inputs();
        for (index, input) in inputs.iter().enumerate() {
            assert!(
                !inputs[..index]
                    .iter()
                    .any(|other| other.value == input.value)
            );
        }
    }
}
