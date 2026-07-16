use std::mem::size_of;
use std::ptr::addr_of;
use std::thread;
use std::time::Duration;

use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitors, GetCapabilitiesStringLength,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR, SetVCPFeature,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::core::BOOL;

use super::{
    INPUT_SOURCE_VCP_CODE, InputCycleReport, MonitorError, MonitorInfo, SwitchOutcome,
    inputs_from_capabilities, is_lg_private_input_model, model_from_capabilities,
};

struct PhysicalMonitorBatch(Vec<PHYSICAL_MONITOR>);

impl Drop for PhysicalMonitorBatch {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitors(&self.0);
        }
    }
}

pub(super) fn scan_monitors() -> Result<Vec<MonitorInfo>, MonitorError> {
    let logical_monitors = enumerate_logical_monitors()?;
    let mut result = Vec::new();

    for (logical_index, logical_monitor) in logical_monitors.into_iter().enumerate() {
        let device_name = monitor_device_name(logical_monitor)
            .unwrap_or_else(|| format!("DISPLAY{}", logical_index + 1));

        let batch = match physical_monitors(logical_monitor) {
            Ok(batch) => batch,
            Err(error) => {
                tracing::warn!(%error, %device_name, "Skipping logical monitor");
                continue;
            }
        };

        for (physical_index, physical) in batch.0.iter().enumerate() {
            let handle = unsafe { addr_of!(physical.hPhysicalMonitor).read_unaligned() };
            let description_buffer =
                unsafe { addr_of!(physical.szPhysicalMonitorDescription).read_unaligned() };
            let description = wide_string(&description_buffer);
            let description = if description.trim().is_empty() {
                format!("Physical monitor {}", physical_index + 1)
            } else {
                description
            };
            let id = monitor_id(&device_name, physical_index, &description);

            let capabilities = capabilities_string(handle);
            if let Some(capabilities) = capabilities.as_deref() {
                tracing::debug!(
                    %device_name,
                    %description,
                    %capabilities,
                    "Monitor capabilities"
                );
            } else {
                tracing::debug!(
                    %device_name,
                    %description,
                    "Monitor did not return a capabilities string"
                );
            }

            let model = capabilities.as_deref().and_then(model_from_capabilities);
            let supported_inputs = capabilities.as_deref().and_then(inputs_from_capabilities);
            let input_state = read_input(handle);

            result.push(MonitorInfo {
                id,
                device_name: device_name.clone(),
                description,
                model,
                supported_inputs,
                current_input: input_state.map(|(current, _)| current),
                maximum_input: input_state.map(|(_, maximum)| maximum),
                ddc_supported: input_state.is_some(),
            });
        }
    }

    Ok(result)
}

pub(super) fn set_input(target_id: &str, input: u32) -> Result<SwitchOutcome, MonitorError> {
    let logical_monitors = enumerate_logical_monitors()?;

    for (logical_index, logical_monitor) in logical_monitors.into_iter().enumerate() {
        let device_name = monitor_device_name(logical_monitor)
            .unwrap_or_else(|| format!("DISPLAY{}", logical_index + 1));
        let batch = match physical_monitors(logical_monitor) {
            Ok(batch) => batch,
            Err(error) => {
                tracing::warn!(%error, %device_name, "Unable to open logical monitor");
                continue;
            }
        };

        for (physical_index, physical) in batch.0.iter().enumerate() {
            let handle = unsafe { addr_of!(physical.hPhysicalMonitor).read_unaligned() };
            let description_buffer =
                unsafe { addr_of!(physical.szPhysicalMonitorDescription).read_unaligned() };
            let description = wide_string(&description_buffer);
            let description = if description.trim().is_empty() {
                format!("Physical monitor {}", physical_index + 1)
            } else {
                description
            };

            if monitor_id(&device_name, physical_index, &description) == target_id {
                let capabilities = capabilities_string(handle);
                let model = capabilities.as_deref().and_then(model_from_capabilities);
                let mut last_reported = None;

                for attempt in 1..=2 {
                    let succeeded =
                        unsafe { SetVCPFeature(handle, INPUT_SOURCE_VCP_CODE, input) != 0 };
                    if !succeeded {
                        return Err(last_error("setting VCP input source"));
                    }

                    thread::sleep(Duration::from_millis(1_200));
                    let input_state = read_input(handle).or_else(|| {
                        thread::sleep(Duration::from_millis(800));
                        read_input(handle)
                    });
                    match input_state {
                        Some((current, _)) if current == input => {
                            tracing::info!(
                                input = format_args!("0x{input:02X}"),
                                attempt,
                                "Verified monitor input switch"
                            );
                            return Ok(SwitchOutcome::Verified);
                        }
                        Some((current, _)) => {
                            last_reported = Some(current);
                            tracing::debug!(
                                requested = format_args!("0x{input:02X}"),
                                reported = format_args!("0x{current:02X}"),
                                attempt,
                                "Monitor has not applied the requested input yet"
                            );
                        }
                        None => {
                            tracing::info!(
                                input = format_args!("0x{input:02X}"),
                                "Monitor stopped answering after accepting the input command"
                            );
                            return Ok(SwitchOutcome::CommandAccepted);
                        }
                    }
                }

                if let Some(alternate_input) = lg_alternate_input(model.as_deref(), input) {
                    tracing::info!(
                        model = model.as_deref().unwrap_or("unknown"),
                        standard_input = format_args!("0x{input:02X}"),
                        alternate_input = format_args!("0x{alternate_input:02X}"),
                        "Falling back to an LG alternate input channel"
                    );
                    send_lg_alternate_input(alternate_input)
                        .map_err(MonitorError::AlternateInput)?;
                    return Ok(SwitchOutcome::AlternateCommandSent);
                }

                return Err(MonitorError::InputNotApplied {
                    requested: input,
                    reported: last_reported.unwrap_or_default(),
                });
            }
        }
    }

    Err(MonitorError::NotFound(target_id.to_owned()))
}

pub(super) fn cycle_standard_input(
    target_id: &str,
    input: u32,
) -> Result<InputCycleReport, MonitorError> {
    let logical_monitors = enumerate_logical_monitors()?;

    for (logical_index, logical_monitor) in logical_monitors.into_iter().enumerate() {
        let device_name = monitor_device_name(logical_monitor)
            .unwrap_or_else(|| format!("DISPLAY{}", logical_index + 1));
        let batch = match physical_monitors(logical_monitor) {
            Ok(batch) => batch,
            Err(error) => {
                tracing::warn!(%error, %device_name, "Unable to open logical monitor for cycle test");
                continue;
            }
        };

        for (physical_index, physical) in batch.0.iter().enumerate() {
            let handle = unsafe { addr_of!(physical.hPhysicalMonitor).read_unaligned() };
            let description_buffer =
                unsafe { addr_of!(physical.szPhysicalMonitorDescription).read_unaligned() };
            let description = wide_string(&description_buffer);
            let description = if description.trim().is_empty() {
                format!("Physical monitor {}", physical_index + 1)
            } else {
                description
            };

            if monitor_id(&device_name, physical_index, &description) != target_id {
                continue;
            }

            let before = read_input(handle)
                .map(|(current, _)| current)
                .ok_or_else(|| MonitorError::InputUnavailable(target_id.to_owned()))?;
            if before == input {
                return Ok(InputCycleReport {
                    before,
                    requested: input,
                    after_target: Some(before),
                    restore_command_accepted: true,
                    after_restore: Some(before),
                });
            }

            let mut target_command_accepted = false;
            let mut after_target = Some(before);
            for _ in 0..2 {
                target_command_accepted |=
                    unsafe { SetVCPFeature(handle, INPUT_SOURCE_VCP_CODE, input) != 0 };
                if !target_command_accepted {
                    return Err(last_error("setting the cycle target input"));
                }

                thread::sleep(Duration::from_millis(1_500));
                after_target = read_input(handle).map(|(current, _)| current);
                if after_target == Some(input) || after_target.is_none() {
                    break;
                }
            }

            if after_target == Some(before) {
                return Ok(InputCycleReport {
                    before,
                    requested: input,
                    after_target,
                    restore_command_accepted: true,
                    after_restore: Some(before),
                });
            }

            thread::sleep(Duration::from_millis(2_500));
            let mut restore_command_accepted = false;
            let mut after_restore = None;
            for _ in 0..3 {
                restore_command_accepted |=
                    unsafe { SetVCPFeature(handle, INPUT_SOURCE_VCP_CODE, before) != 0 };
                thread::sleep(Duration::from_millis(1_500));
                after_restore = read_input(handle).map(|(current, _)| current);
                if after_restore == Some(before) {
                    break;
                }
            }

            return Ok(InputCycleReport {
                before,
                requested: input,
                after_target,
                restore_command_accepted,
                after_restore,
            });
        }
    }

    Err(MonitorError::NotFound(target_id.to_owned()))
}

fn capabilities_string(handle: windows::Win32::Foundation::HANDLE) -> Option<String> {
    let mut length = 0;
    if unsafe { GetCapabilitiesStringLength(handle, &mut length) } == 0 || length == 0 {
        return None;
    }

    let mut buffer = vec![0; length as usize];
    if unsafe { CapabilitiesRequestAndCapabilitiesReply(handle, &mut buffer) } == 0 {
        return None;
    }

    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

fn read_input(handle: windows::Win32::Foundation::HANDLE) -> Option<(u32, u32)> {
    let mut current = 0;
    let mut maximum = 0;
    let succeeded = unsafe {
        GetVCPFeatureAndVCPFeatureReply(
            handle,
            INPUT_SOURCE_VCP_CODE,
            None,
            &mut current,
            Some(&mut maximum),
        ) != 0
    };
    succeeded.then_some((current, maximum))
}

fn lg_alternate_input(model: Option<&str>, standard_input: u32) -> Option<u8> {
    if !is_lg_private_input_model(model) {
        return None;
    }

    match standard_input {
        0x0f => Some(0xd0),
        0x10 => Some(0xd2),
        0x11 => Some(0x90),
        0x12 => Some(0x91),
        _ => None,
    }
}

fn send_lg_alternate_input(value: u8) -> Result<(), String> {
    let mut errors = Vec::new();

    if super::lg_usb::is_available() {
        match super::lg_usb::send_input(value) {
            Ok(response) => {
                if let Some(error) = response.response_error {
                    tracing::warn!(
                        %error,
                        bytes_written = response.bytes_written,
                        "LG USB alternate input command did not return a usable bridge response"
                    );
                    errors.push(format!("USB HID: {error}"));
                } else {
                    tracing::info!(
                        bytes_written = response.bytes_written,
                        response_bytes = response.response.len(),
                        "Sent LG alternate input command through USB HID"
                    );
                    return Ok(());
                }
            }
            Err(error) => {
                tracing::warn!(%error, "LG USB alternate input command failed");
                errors.push(format!("USB HID: {error}"));
            }
        }
    }

    match super::lg_intel::request_elevated_switch(value) {
        Ok(()) => Ok(()),
        Err(error) => {
            errors.push(format!("Intel IGCL: {error}"));
            Err(errors.join("; "))
        }
    }
}

fn enumerate_logical_monitors() -> Result<Vec<HMONITOR>, MonitorError> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _device_context: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        BOOL(1)
    }

    let mut monitors = Vec::new();
    let succeeded = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM((&mut monitors as *mut Vec<HMONITOR>) as isize),
        )
        .as_bool()
    };

    if succeeded {
        Ok(monitors)
    } else {
        Err(last_error("enumerating logical monitors"))
    }
}

fn physical_monitors(logical_monitor: HMONITOR) -> Result<PhysicalMonitorBatch, MonitorError> {
    let mut count = 0;
    unsafe {
        GetNumberOfPhysicalMonitorsFromHMONITOR(logical_monitor, &mut count)
            .map_err(|error| api_error("querying physical monitor count", error.message()))?;
    }

    if count == 0 {
        return Ok(PhysicalMonitorBatch(Vec::new()));
    }

    let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    unsafe {
        GetPhysicalMonitorsFromHMONITOR(logical_monitor, &mut monitors)
            .map_err(|error| api_error("opening physical monitors", error.message()))?;
    }
    Ok(PhysicalMonitorBatch(monitors))
}

fn monitor_device_name(monitor: HMONITOR) -> Option<String> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let succeeded = unsafe { GetMonitorInfoW(monitor, &raw mut info.monitorInfo).as_bool() };
    succeeded.then(|| wide_string(&info.szDevice))
}

fn monitor_id(device_name: &str, physical_index: usize, description: &str) -> String {
    format!("{device_name}::{physical_index}::{description}")
}

fn wide_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

fn last_error(operation: &'static str) -> MonitorError {
    api_error(operation, windows::core::Error::from_thread().message())
}

fn api_error(operation: &'static str, message: impl Into<String>) -> MonitorError {
    MonitorError::WindowsApi {
        operation,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_wk95u_standard_inputs_to_lg_side_channel() {
        assert_eq!(lg_alternate_input(Some("WK95U"), 0x0f), Some(0xd0));
        assert_eq!(lg_alternate_input(Some("WK95U"), 0x10), Some(0xd2));
        assert_eq!(lg_alternate_input(Some("WK95U"), 0x11), Some(0x90));
        assert_eq!(lg_alternate_input(Some("WK95U"), 0x12), Some(0x91));
    }

    #[test]
    fn does_not_guess_side_channel_values_for_other_models() {
        assert_eq!(lg_alternate_input(Some("UN880"), 0x11), None);
        assert_eq!(lg_alternate_input(None, 0x11), None);
    }
}
