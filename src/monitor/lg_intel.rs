use std::{
    ffi::{OsStr, c_void},
    mem::size_of,
    os::windows::ffi::OsStrExt,
    ptr, thread,
    time::Duration,
};

use libloading::Library;
use windows::{
    Win32::{
        Foundation::{CloseHandle, WAIT_FAILED},
        System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::{PCWSTR, w},
};

const CTL_RESULT_SUCCESS: i32 = 0;
const CTL_RESULT_ERROR_INSUFFICIENT_PERMISSIONS: i32 = 0x4000_0006;
const DISPLAY_ATTACHED: u32 = 0x2;
const INTERNAL_DISPLAY: u32 = 0x1;
const HELPER_ARGUMENT: &str = "--lg-intel-alt";
const HELPER_CYCLE_ARGUMENT: &str = "--lg-intel-cycle";
const HELPER_VCP_ARGUMENT: &str = "--lg-intel-vcp";
const HELPER_VCP_CYCLE_ARGUMENT: &str = "--lg-intel-vcp-cycle";

const EXIT_INVALID_ARGUMENT: i32 = 2;
const EXIT_LIBRARY_UNAVAILABLE: i32 = 3;
const EXIT_INITIALIZATION_FAILED: i32 = 4;
const EXIT_NO_EXTERNAL_DISPLAY: i32 = 5;
const EXIT_MULTIPLE_EXTERNAL_DISPLAYS: i32 = 6;
const EXIT_WRITE_FAILED: i32 = 7;
const EXIT_INSUFFICIENT_PERMISSIONS: i32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlApplicationId {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlInitArgs {
    size: u32,
    version: u8,
    app_version: u32,
    flags: u32,
    supported_version: u32,
    application_uid: CtlApplicationId,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlOsDisplayEncoderIdentifier {
    raw: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlDisplayTiming {
    size: u32,
    version: u8,
    pixel_clock: u64,
    h_active: u32,
    v_active: u32,
    h_total: u32,
    v_total: u32,
    h_blank: u32,
    v_blank: u32,
    h_sync: u32,
    v_sync: u32,
    refresh_rate: f32,
    signal_standard: i32,
    vic_id: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlDisplayProperties {
    size: u32,
    version: u8,
    os_display_encoder_handle: CtlOsDisplayEncoderIdentifier,
    output_type: i32,
    attached_display_mux_type: i32,
    protocol_converter_output: i32,
    supported_spec_major: u8,
    supported_spec_minor: u8,
    supported_spec_revision: u8,
    supported_output_bpc_flags: u32,
    protocol_converter_type: u32,
    display_config_flags: u32,
    feature_enabled_flags: u32,
    feature_supported_flags: u32,
    advanced_feature_enabled_flags: u32,
    advanced_feature_supported_flags: u32,
    display_timing_info: CtlDisplayTiming,
    reserved_fields: [u32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CtlAdapterDisplayEncoderProperties {
    size: u32,
    version: u8,
    os_display_encoder_handle: CtlOsDisplayEncoderIdentifier,
    output_type: i32,
    is_onboard_protocol_converter_output_present: u8,
    supported_spec_major: u8,
    supported_spec_minor: u8,
    supported_spec_revision: u8,
    supported_output_bpc_flags: u32,
    encoder_config_flags: u32,
    feature_supported_flags: u32,
    advanced_feature_supported_flags: u32,
    reserved_fields: [u32; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CtlI2cAccessArgs {
    size: u32,
    version: u8,
    data_size: u32,
    address: u32,
    operation_type: i32,
    offset: u32,
    flags: u32,
    rad: u64,
    data: [u8; 128],
}

impl Default for CtlI2cAccessArgs {
    fn default() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            version: 0,
            data_size: 0,
            address: 0,
            operation_type: 0,
            offset: 0,
            flags: 0,
            rad: 0,
            data: [0; 128],
        }
    }
}

type CtlInit = unsafe extern "C" fn(*mut CtlInitArgs, *mut *mut c_void) -> i32;
type CtlClose = unsafe extern "C" fn(*mut c_void) -> i32;
type CtlEnumerateDevices = unsafe extern "C" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32;
type CtlEnumerateDisplayOutputs =
    unsafe extern "C" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32;
type CtlGetDisplayProperties = unsafe extern "C" fn(*mut c_void, *mut CtlDisplayProperties) -> i32;
type CtlGetAdapterDisplayEncoderProperties =
    unsafe extern "C" fn(*mut c_void, *mut CtlAdapterDisplayEncoderProperties) -> i32;
type CtlI2cAccess = unsafe extern "C" fn(*mut c_void, *mut CtlI2cAccessArgs) -> i32;

struct ApiHandle {
    handle: *mut c_void,
    close: CtlClose,
}

impl Drop for ApiHandle {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.handle);
        }
    }
}

pub(super) fn run_helper_command() -> Option<i32> {
    let mut arguments = std::env::args().skip(1);
    let operation = match arguments.next().as_deref() {
        Some(HELPER_ARGUMENT) => {
            let Some(value) = arguments.next().and_then(|value| parse_byte(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            send_input(value)
        }
        Some(HELPER_CYCLE_ARGUMENT) => {
            let Some(target) = arguments.next().and_then(|value| parse_byte(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            let Some(restore) = arguments.next().and_then(|value| parse_byte(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            cycle_inputs(target, restore)
        }
        Some(HELPER_VCP_ARGUMENT) => {
            let Some(vcp_code) = arguments.next().and_then(|value| parse_byte(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            let Some(value) = arguments.next().and_then(|value| parse_word(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            send_vcp(vcp_code, value)
        }
        Some(HELPER_VCP_CYCLE_ARGUMENT) => {
            let Some(vcp_code) = arguments.next().and_then(|value| parse_byte(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            let Some(target) = arguments.next().and_then(|value| parse_word(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            let Some(restore) = arguments.next().and_then(|value| parse_word(&value)) else {
                return Some(EXIT_INVALID_ARGUMENT);
            };
            cycle_vcp(vcp_code, target, restore)
        }
        _ => return None,
    };

    Some(helper_exit_code(operation))
}

fn helper_exit_code(result: Result<(), HelperError>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(HelperError::LibraryUnavailable(error)) => {
            tracing::error!(%error, "Intel Graphics Control Library is unavailable");
            EXIT_LIBRARY_UNAVAILABLE
        }
        Err(HelperError::InitializationFailed(status)) => {
            tracing::error!(
                status = format_args!("0x{status:08X}"),
                "Intel graphics control initialization failed"
            );
            EXIT_INITIALIZATION_FAILED
        }
        Err(HelperError::NoExternalDisplay) => EXIT_NO_EXTERNAL_DISPLAY,
        Err(HelperError::MultipleExternalDisplays(count)) => {
            tracing::error!(
                count,
                "Cannot safely choose among multiple Intel external displays"
            );
            EXIT_MULTIPLE_EXTERNAL_DISPLAYS
        }
        Err(HelperError::Api(operation, CTL_RESULT_ERROR_INSUFFICIENT_PERMISSIONS)) => {
            tracing::error!(
                operation,
                "Intel I2C access requires administrator permission"
            );
            EXIT_INSUFFICIENT_PERMISSIONS
        }
        Err(HelperError::Api(operation, status)) => {
            tracing::error!(
                operation,
                status = format_args!("0x{status:08X}"),
                "Intel I2C input switch failed"
            );
            EXIT_WRITE_FAILED
        }
    }
}

pub(super) fn request_elevated_switch(value: u8) -> Result<(), String> {
    request_elevated_helper(
        &format!("{HELPER_ARGUMENT} 0x{value:02X}"),
        "The elevated LG input helper did not start.",
    )
}

pub(super) fn request_elevated_cycle(target: u8, restore: u8) -> Result<(), String> {
    request_elevated_helper(
        &format!("{HELPER_CYCLE_ARGUMENT} 0x{target:02X} 0x{restore:02X}"),
        "The elevated LG input cycle helper did not start.",
    )
}

pub(super) fn request_elevated_vcp(vcp_code: u8, value: u16) -> Result<(), String> {
    request_elevated_helper(
        &format!("{HELPER_VCP_ARGUMENT} 0x{vcp_code:02X} 0x{value:04X}"),
        "The elevated LG VCP helper did not start.",
    )
}

pub(super) fn request_elevated_vcp_cycle(
    vcp_code: u8,
    target: u16,
    restore: u16,
) -> Result<(), String> {
    request_elevated_helper(
        &format!("{HELPER_VCP_CYCLE_ARGUMENT} 0x{vcp_code:02X} 0x{target:04X} 0x{restore:04X}"),
        "The elevated LG VCP cycle helper did not start.",
    )
}

fn request_elevated_helper(parameters: &str, startup_error: &str) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the application executable: {error}"))?;
    let executable = wide(executable.as_os_str());
    let parameters = wide(OsStr::new(parameters));

    let mut execute_info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&raw mut execute_info) }
        .map_err(|error| format!("Administrator approval was not granted: {error}"))?;

    let process = execute_info.hProcess;
    if process.is_invalid() {
        return Err(startup_error.into());
    }

    let wait_result = unsafe { WaitForSingleObject(process, INFINITE) };
    if wait_result == WAIT_FAILED {
        unsafe {
            let _ = CloseHandle(process);
        }
        return Err("Waiting for the elevated LG input helper failed.".into());
    }

    let mut exit_code = EXIT_WRITE_FAILED as u32;
    let exit_result = unsafe { GetExitCodeProcess(process, &mut exit_code) };
    unsafe {
        let _ = CloseHandle(process);
    }
    exit_result.map_err(|error| format!("Could not read the LG input helper result: {error}"))?;

    match exit_code as i32 {
        0 => Ok(()),
        EXIT_INVALID_ARGUMENT => Err("The LG alternate input value was invalid.".into()),
        EXIT_LIBRARY_UNAVAILABLE => Err(
            "Intel ControlLib.dll is unavailable. Install or update the Intel graphics driver."
                .into(),
        ),
        EXIT_INITIALIZATION_FAILED => {
            Err("Intel Graphics Control Library initialization failed.".into())
        }
        EXIT_NO_EXTERNAL_DISPLAY => {
            Err("Intel Graphics Control Library did not find an attached external display.".into())
        }
        EXIT_MULTIPLE_EXTERNAL_DISPLAYS => Err(
            "Multiple external displays are attached to Intel graphics, so the LG alternate command cannot be targeted safely."
                .into(),
        ),
        EXIT_WRITE_FAILED => Err(
            "Intel Graphics Control Library could not send the LG alternate input command."
                .into(),
        ),
        EXIT_INSUFFICIENT_PERMISSIONS => Err(
            "Intel Graphics Control Library still denied I2C access after elevation.".into(),
        ),
        code => Err(format!("The LG alternate input helper failed with exit code {code}.")),
    }
}

fn send_input(value: u8) -> Result<(), HelperError> {
    with_external_display(|display, i2c_access| write_input_twice(display, value, i2c_access))
}

fn send_vcp(vcp_code: u8, value: u16) -> Result<(), HelperError> {
    with_external_display(|display, i2c_access| {
        write_vcp_twice(display, vcp_code, value, i2c_access)
    })
}

fn cycle_inputs(target: u8, restore: u8) -> Result<(), HelperError> {
    with_external_display(|display, i2c_access| {
        write_input_twice(display, target, i2c_access)?;
        thread::sleep(Duration::from_secs(8));
        write_input_twice(display, restore, i2c_access)
    })
}

fn cycle_vcp(vcp_code: u8, target: u16, restore: u16) -> Result<(), HelperError> {
    with_external_display(|display, i2c_access| {
        write_vcp_twice(display, vcp_code, target, i2c_access)?;
        thread::sleep(Duration::from_secs(8));
        write_vcp_twice(display, vcp_code, restore, i2c_access)
    })
}

fn with_external_display<T>(
    operation: impl FnOnce(*mut c_void, CtlI2cAccess) -> Result<T, HelperError>,
) -> Result<T, HelperError> {
    let library = unsafe { Library::new("ControlLib.dll") }
        .map_err(|error| HelperError::LibraryUnavailable(error.to_string()))?;
    let ctl_init: CtlInit = load(&library, b"ctlInit\0")?;
    let ctl_close: CtlClose = load(&library, b"ctlClose\0")?;
    let enumerate_devices: CtlEnumerateDevices = load(&library, b"ctlEnumerateDevices\0")?;
    let enumerate_outputs: CtlEnumerateDisplayOutputs =
        load(&library, b"ctlEnumerateDisplayOutputs\0")?;
    let get_display_properties: CtlGetDisplayProperties =
        load(&library, b"ctlGetDisplayProperties\0")?;
    let get_encoder_properties: CtlGetAdapterDisplayEncoderProperties =
        load(&library, b"ctlGetAdaperDisplayEncoderProperties\0")?;
    let i2c_access: CtlI2cAccess = load(&library, b"ctlI2CAccess\0")?;

    let mut init_args = CtlInitArgs {
        size: size_of::<CtlInitArgs>() as u32,
        app_version: (1 << 16) | 1,
        ..Default::default()
    };
    let mut raw_handle = ptr::null_mut();
    let status = unsafe { ctl_init(&raw mut init_args, &raw mut raw_handle) };
    if status != CTL_RESULT_SUCCESS {
        return Err(HelperError::InitializationFailed(status));
    }
    let _api_handle = ApiHandle {
        handle: raw_handle,
        close: ctl_close,
    };

    let displays = external_displays(
        raw_handle,
        enumerate_devices,
        enumerate_outputs,
        get_display_properties,
        get_encoder_properties,
    )?;
    let display = match displays.as_slice() {
        [] => return Err(HelperError::NoExternalDisplay),
        [display] => *display,
        displays => return Err(HelperError::MultipleExternalDisplays(displays.len())),
    };

    operation(display, i2c_access)
}

fn write_input_twice(
    display: *mut c_void,
    value: u8,
    i2c_access: CtlI2cAccess,
) -> Result<(), HelperError> {
    for attempt in 0..2 {
        write_input(display, value, i2c_access)?;
        if attempt == 0 {
            thread::sleep(Duration::from_millis(1_200));
        }
    }

    Ok(())
}

fn write_vcp_twice(
    display: *mut c_void,
    vcp_code: u8,
    value: u16,
    i2c_access: CtlI2cAccess,
) -> Result<(), HelperError> {
    for attempt in 0..2 {
        write_vcp(display, vcp_code, value, i2c_access)?;
        if attempt == 0 {
            thread::sleep(Duration::from_millis(1_200));
        }
    }

    Ok(())
}

fn external_displays(
    api_handle: *mut c_void,
    enumerate_devices: CtlEnumerateDevices,
    enumerate_outputs: CtlEnumerateDisplayOutputs,
    get_display_properties: CtlGetDisplayProperties,
    get_encoder_properties: CtlGetAdapterDisplayEncoderProperties,
) -> Result<Vec<*mut c_void>, HelperError> {
    let mut device_count = 0;
    api_call("enumerating Intel graphics adapters", unsafe {
        enumerate_devices(api_handle, &raw mut device_count, ptr::null_mut())
    })?;
    let mut devices = vec![ptr::null_mut(); device_count as usize];
    api_call("opening Intel graphics adapters", unsafe {
        enumerate_devices(api_handle, &raw mut device_count, devices.as_mut_ptr())
    })?;

    let mut result = Vec::new();
    for device in devices {
        let mut display_count = 0;
        api_call("enumerating Intel display outputs", unsafe {
            enumerate_outputs(device, &raw mut display_count, ptr::null_mut())
        })?;
        let mut displays = vec![ptr::null_mut(); display_count as usize];
        api_call("opening Intel display outputs", unsafe {
            enumerate_outputs(device, &raw mut display_count, displays.as_mut_ptr())
        })?;

        for display in displays {
            let mut display_properties = CtlDisplayProperties {
                size: size_of::<CtlDisplayProperties>() as u32,
                ..Default::default()
            };
            if unsafe { get_display_properties(display, &raw mut display_properties) }
                != CTL_RESULT_SUCCESS
                || display_properties.display_config_flags & DISPLAY_ATTACHED == 0
            {
                continue;
            }

            let mut encoder_properties = CtlAdapterDisplayEncoderProperties {
                size: size_of::<CtlAdapterDisplayEncoderProperties>() as u32,
                ..Default::default()
            };
            if unsafe { get_encoder_properties(display, &raw mut encoder_properties) }
                != CTL_RESULT_SUCCESS
                || encoder_properties.encoder_config_flags & INTERNAL_DISPLAY != 0
            {
                continue;
            }

            result.push(display);
        }
    }

    Ok(result)
}

fn write_input(
    display: *mut c_void,
    value: u8,
    i2c_access: CtlI2cAccess,
) -> Result<(), HelperError> {
    let frame = super::lg_ddc::lg_input_set_vcp_frame(value);
    write_frame(
        display,
        &frame,
        "writing the LG alternate input command",
        i2c_access,
    )
}

fn write_vcp(
    display: *mut c_void,
    vcp_code: u8,
    value: u16,
    i2c_access: CtlI2cAccess,
) -> Result<(), HelperError> {
    let frame = super::lg_ddc::lg_source_set_vcp_frame(vcp_code, value);
    write_frame(
        display,
        &frame,
        "writing the LG raw VCP command",
        i2c_access,
    )
}

fn write_frame(
    display: *mut c_void,
    frame: &[u8],
    operation: &'static str,
    i2c_access: CtlI2cAccess,
) -> Result<(), HelperError> {
    let mut arguments = CtlI2cAccessArgs {
        data_size: frame.len() as u32,
        address: super::lg_ddc::DDC_WRITE_ADDRESS as u32,
        operation_type: 2,
        ..Default::default()
    };
    arguments.data[..frame.len()].copy_from_slice(frame);

    api_call(operation, unsafe {
        i2c_access(display, &raw mut arguments)
    })
}

fn api_call(operation: &'static str, status: i32) -> Result<(), HelperError> {
    if status == CTL_RESULT_SUCCESS {
        Ok(())
    } else {
        Err(HelperError::Api(operation, status))
    }
}

fn load<T: Copy>(library: &Library, symbol: &[u8]) -> Result<T, HelperError> {
    unsafe { library.get::<T>(symbol) }
        .map(|symbol| *symbol)
        .map_err(|error| HelperError::LibraryUnavailable(error.to_string()))
}

fn parse_byte(value: &str) -> Option<u8> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .and_then(|value| u8::from_str_radix(value, 16).ok())
        .or_else(|| value.parse().ok())
}

fn parse_word(value: &str) -> Option<u16> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .and_then(|value| u16::from_str_radix(value, 16).ok())
        .or_else(|| value.parse().ok())
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

enum HelperError {
    LibraryUnavailable(String),
    InitializationFailed(i32),
    NoExternalDisplay,
    MultipleExternalDisplays(usize),
    Api(&'static str, i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hexadecimal_helper_values() {
        assert_eq!(parse_byte("0xD2"), Some(0xd2));
        assert_eq!(parse_byte("144"), Some(0x90));
        assert_eq!(parse_byte("input"), None);
        assert_eq!(parse_word("0x1234"), Some(0x1234));
    }

    #[test]
    fn igcl_struct_layout_matches_control_library_abi() {
        assert_eq!(size_of::<CtlInitArgs>(), 36);
        assert_eq!(size_of::<CtlDisplayTiming>(), 64);
        assert_eq!(size_of::<CtlDisplayProperties>(), 200);
        assert_eq!(size_of::<CtlAdapterDisplayEncoderProperties>(), 112);
        assert_eq!(size_of::<CtlI2cAccessArgs>(), 168);
    }
}
