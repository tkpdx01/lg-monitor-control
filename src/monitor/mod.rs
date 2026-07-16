use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(windows)]
mod lg_cdc;
#[cfg(windows)]
mod lg_ddc;
#[cfg(windows)]
mod lg_intel;
#[cfg(windows)]
mod lg_usb;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputCycleReport {
    pub before: u32,
    pub requested: u32,
    pub after_target: Option<u32>,
    pub restore_command_accepted: bool,
    pub after_restore: Option<u32>,
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
    #[error("Monitor '{0}' stopped reporting its current input before the test started")]
    InputUnavailable(String),
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

pub(crate) fn is_lg_private_input_model(model: Option<&str>) -> bool {
    model.is_some_and(|model| {
        let model = model.to_ascii_uppercase();
        model.contains("WK95U") || model.contains("BK95U")
    })
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
pub fn run_cli_command() -> Option<i32> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("--list-monitors") => Some(match scan_monitors() {
            Ok(monitors) => {
                for (index, monitor) in monitors.iter().enumerate() {
                    println!(
                        "[{index}] id={:?} device={:?} description={:?} model={} current={} maximum={} inputs={} ddc={}",
                        monitor.id,
                        monitor.device_name,
                        monitor.description,
                        monitor.model.as_deref().unwrap_or("unknown"),
                        format_optional_hex(monitor.current_input),
                        format_optional_hex(monitor.maximum_input),
                        monitor
                            .supported_inputs
                            .as_deref()
                            .map(format_hex_values)
                            .unwrap_or_else(|| "unknown".into()),
                        monitor.ddc_supported,
                    );
                }
                println!("lg_usb_control={}", lg_usb::is_available());
                println!("lg_cdc_control={}", lg_cdc::is_available());
                0
            }
            Err(error) => {
                eprintln!("Monitor scan failed: {error}");
                1
            }
        }),
        Some("--switch-input") => {
            let Some(monitor_index) = arguments
                .next()
                .and_then(|value| value.parse::<usize>().ok())
            else {
                eprintln!("Usage: --switch-input <monitor-index> <hex-or-decimal-input>");
                return Some(2);
            };
            let Some(input) = arguments.next().and_then(|value| parse_cli_value(&value)) else {
                eprintln!("Usage: --switch-input <monitor-index> <hex-or-decimal-input>");
                return Some(2);
            };

            Some(match scan_monitors() {
                Ok(monitors) => {
                    let Some(monitor) = monitors.get(monitor_index) else {
                        eprintln!(
                            "Monitor index {monitor_index} does not exist; {} monitor(s) were found.",
                            monitors.len()
                        );
                        return Some(2);
                    };
                    println!(
                        "Switching monitor {:?} ({}) from {} to 0x{input:02X}",
                        monitor.description,
                        monitor.model.as_deref().unwrap_or("unknown"),
                        format_optional_hex(monitor.current_input),
                    );
                    match set_input(&monitor.id, input) {
                        Ok(outcome) => {
                            println!("Switch outcome: {outcome:?}");
                            0
                        }
                        Err(error) => {
                            eprintln!("Switch failed: {error}");
                            1
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Monitor scan failed: {error}");
                    1
                }
            })
        }
        Some("--lg-usb-alt") => {
            let Some(value) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-usb-alt <hex-or-decimal-private-input>");
                return Some(2);
            };

            Some(match lg_usb::send_input(value) {
                Ok(response) => {
                    if let Some(error) = response.response_error {
                        eprintln!(
                            "LG USB command value=0x{value:02X} bytes_written={} response=unavailable ({error})",
                            response.bytes_written,
                        );
                        1
                    } else {
                        println!(
                            "LG USB command value=0x{value:02X} bytes_written={} response={}",
                            response.bytes_written,
                            format_hex_bytes(&response.response),
                        );
                        0
                    }
                }
                Err(error) => {
                    eprintln!("LG USB command failed: {error}");
                    1
                }
            })
        }
        Some("--lg-cdc-get") => {
            let Some(vcp_code) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-cdc-get <hex-or-decimal-vcp-code>");
                return Some(2);
            };

            Some(match lg_cdc::get_vcp(vcp_code) {
                Ok(response) => {
                    println!(
                        "LG CDC get vcp=0x{vcp_code:02X} port={} bytes_written={} response={}",
                        response.port_name,
                        response.bytes_written,
                        format_hex_bytes(&response.response),
                    );
                    0
                }
                Err(error) => {
                    eprintln!("LG CDC get failed: {error}");
                    1
                }
            })
        }
        Some("--lg-cdc-set") => {
            let Some(vcp_code) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-cdc-set <vcp-code> <value>");
                return Some(2);
            };
            let Some(value) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u16::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-cdc-set <vcp-code> <value>");
                return Some(2);
            };

            Some(match lg_cdc::set_vcp(vcp_code, value) {
                Ok(response) => {
                    println!(
                        "LG CDC set vcp=0x{vcp_code:02X} value=0x{value:04X} port={} bytes_written={} response={}",
                        response.port_name,
                        response.bytes_written,
                        format_hex_bytes(&response.response),
                    );
                    0
                }
                Err(error) => {
                    eprintln!("LG CDC set failed: {error}");
                    1
                }
            })
        }
        Some("--lg-cdc-alt") => {
            let Some(value) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u16::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-cdc-alt <hex-or-decimal-private-input>");
                return Some(2);
            };

            Some(match lg_cdc::set_vcp(lg_ddc::LG_INPUT_VCP_CODE, value) {
                Ok(response) => {
                    println!(
                        "LG CDC alt value=0x{value:04X} port={} bytes_written={} response={}",
                        response.port_name,
                        response.bytes_written,
                        format_hex_bytes(&response.response),
                    );
                    0
                }
                Err(error) => {
                    eprintln!("LG CDC alt failed: {error}");
                    1
                }
            })
        }
        Some("--lg-cdc-raw") => {
            let payload = arguments
                .filter_map(|value| {
                    parse_cli_value(&value).and_then(|value| u8::try_from(value).ok())
                })
                .collect::<Vec<_>>();
            if payload.is_empty() {
                eprintln!("Usage: --lg-cdc-raw <payload-byte> [payload-byte...]");
                return Some(2);
            }

            Some(match lg_cdc::send_raw(&payload) {
                Ok(response) => {
                    println!(
                        "LG CDC raw payload={} port={} bytes_written={} response={}",
                        format_hex_bytes(&payload),
                        response.port_name,
                        response.bytes_written,
                        format_hex_bytes(&response.response),
                    );
                    0
                }
                Err(error) => {
                    eprintln!("LG CDC raw failed: {error}");
                    1
                }
            })
        }
        Some("--lg-usb-cycle") => {
            let Some(target) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-usb-cycle <target> <restore>");
                return Some(2);
            };
            let Some(restore) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-usb-cycle <target> <restore>");
                return Some(2);
            };

            Some(match lg_usb::cycle_inputs(target, restore) {
                Ok(()) => {
                    println!(
                        "LG USB cycle target=0x{target:02X} restore=0x{restore:02X} completed"
                    );
                    0
                }
                Err(error) => {
                    eprintln!("LG USB cycle failed: {error}");
                    1
                }
            })
        }
        Some("--lg-intel-elevated-cycle") => {
            let Some(target) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-cycle <target> <restore>");
                return Some(2);
            };
            let Some(restore) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-cycle <target> <restore>");
                return Some(2);
            };

            Some(match lg_intel::request_elevated_cycle(target, restore) {
                Ok(()) => {
                    println!(
                        "LG Intel cycle target=0x{target:02X} restore=0x{restore:02X} completed"
                    );
                    0
                }
                Err(error) => {
                    eprintln!("LG Intel cycle failed: {error}");
                    1
                }
            })
        }
        Some("--lg-intel-elevated-vcp") => {
            let Some(vcp_code) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-vcp <vcp-code> <value>");
                return Some(2);
            };
            let Some(value) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u16::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-vcp <vcp-code> <value>");
                return Some(2);
            };

            Some(match lg_intel::request_elevated_vcp(vcp_code, value) {
                Ok(()) => {
                    println!("LG Intel VCP vcp=0x{vcp_code:02X} value=0x{value:04X} completed");
                    0
                }
                Err(error) => {
                    eprintln!("LG Intel VCP failed: {error}");
                    1
                }
            })
        }
        Some("--lg-intel-elevated-vcp-cycle") => {
            let Some(vcp_code) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u8::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-vcp-cycle <vcp-code> <target> <restore>");
                return Some(2);
            };
            let Some(target) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u16::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-vcp-cycle <vcp-code> <target> <restore>");
                return Some(2);
            };
            let Some(restore) = arguments
                .next()
                .and_then(|value| parse_cli_value(&value))
                .and_then(|value| u16::try_from(value).ok())
            else {
                eprintln!("Usage: --lg-intel-elevated-vcp-cycle <vcp-code> <target> <restore>");
                return Some(2);
            };

            Some(
                match lg_intel::request_elevated_vcp_cycle(vcp_code, target, restore) {
                    Ok(()) => {
                        println!(
                            "LG Intel VCP cycle vcp=0x{vcp_code:02X} target=0x{target:04X} restore=0x{restore:04X} completed"
                        );
                        0
                    }
                    Err(error) => {
                        eprintln!("LG Intel VCP cycle failed: {error}");
                        1
                    }
                },
            )
        }
        Some("--cycle-input") => {
            let Some(monitor_index) = arguments
                .next()
                .and_then(|value| value.parse::<usize>().ok())
            else {
                eprintln!("Usage: --cycle-input <monitor-index> <hex-or-decimal-input>");
                return Some(2);
            };
            let Some(input) = arguments.next().and_then(|value| parse_cli_value(&value)) else {
                eprintln!("Usage: --cycle-input <monitor-index> <hex-or-decimal-input>");
                return Some(2);
            };

            Some(match scan_monitors() {
                Ok(monitors) => {
                    let Some(monitor) = monitors.get(monitor_index) else {
                        eprintln!(
                            "Monitor index {monitor_index} does not exist; {} monitor(s) were found.",
                            monitors.len()
                        );
                        return Some(2);
                    };
                    match windows::cycle_standard_input(&monitor.id, input) {
                        Ok(report) => {
                            println!(
                                "Cycle before=0x{:02X} requested=0x{:02X} after_target={} restore_command={} after_restore={}",
                                report.before,
                                report.requested,
                                format_optional_hex(report.after_target),
                                report.restore_command_accepted,
                                format_optional_hex(report.after_restore),
                            );
                            if report.after_restore == Some(report.before) {
                                0
                            } else {
                                eprintln!(
                                    "The original input could not be verified after the cycle."
                                );
                                1
                            }
                        }
                        Err(error) => {
                            eprintln!("Input cycle failed: {error}");
                            1
                        }
                    }
                }
                Err(error) => {
                    eprintln!("Monitor scan failed: {error}");
                    1
                }
            })
        }
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn run_cli_command() -> Option<i32> {
    None
}

fn parse_cli_value(value: &str) -> Option<u32> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .or_else(|| value.parse().ok())
}

fn format_optional_hex(value: Option<u32>) -> String {
    value
        .map(|value| format!("0x{value:02X}"))
        .unwrap_or_else(|| "unknown".into())
}

fn format_hex_bytes(values: &[u8]) -> String {
    if values.is_empty() {
        return "none".into();
    }

    values
        .iter()
        .map(|value| format!("{value:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_hex_values(values: &[u32]) -> String {
    values
        .iter()
        .map(|value| format!("0x{value:02X}"))
        .collect::<Vec<_>>()
        .join(",")
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
    fn parses_cli_input_values() {
        assert_eq!(parse_cli_value("0x10"), Some(0x10));
        assert_eq!(parse_cli_value("17"), Some(17));
        assert_eq!(parse_cli_value("HDMI"), None);
    }

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

    #[test]
    fn identifies_lg_private_input_models() {
        assert!(is_lg_private_input_model(Some("WK95U")));
        assert!(is_lg_private_input_model(Some("34BK95U-W")));
        assert!(!is_lg_private_input_model(Some("UN880")));
        assert!(!is_lg_private_input_model(None));
    }
}
