use std::{thread, time::Duration};

use hidapi::{HidApi, HidDevice};

const LG_VENDOR_ID: u16 = 0x043e;
const LG_CONTROL_PRODUCT_ID: u16 = 0x9a39;
const REPORT_SIZE: usize = 64;
const WINDOWS_REPORT_SIZE_WITH_ID: usize = REPORT_SIZE + 1;
const RESPONSE_READ_SIZE: u8 = 0x26;
const MAX_RESPONSE_CHUNK_SIZE: u8 = 0x10;

#[derive(Debug)]
pub(super) struct UsbSwitchResponse {
    pub bytes_written: usize,
    pub response: Vec<u8>,
    pub response_error: Option<String>,
}

pub(super) fn is_available() -> bool {
    HidApi::new().is_ok_and(|api| {
        api.device_list().any(|device| {
            device.vendor_id() == LG_VENDOR_ID && device.product_id() == LG_CONTROL_PRODUCT_ID
        })
    })
}

pub(super) fn send_input(value: u8) -> Result<UsbSwitchResponse, String> {
    let api = HidApi::new().map_err(|error| format!("Could not initialize HID access: {error}"))?;
    let matches = api
        .device_list()
        .filter(|device| {
            device.vendor_id() == LG_VENDOR_ID && device.product_id() == LG_CONTROL_PRODUCT_ID
        })
        .collect::<Vec<_>>();

    let device_info = match matches.as_slice() {
        [] => {
            return Err(format!(
                "LG USB control device {:04X}:{:04X} is not connected",
                LG_VENDOR_ID, LG_CONTROL_PRODUCT_ID
            ));
        }
        [device] => *device,
        devices => {
            return Err(format!(
                "{} LG USB control devices are connected; refusing to guess the target",
                devices.len()
            ));
        }
    };
    let device = device_info
        .open_device(&api)
        .map_err(|error| format!("Could not open the LG USB control device: {error}"))?;

    let command = command_report(value);
    let command_size = device
        .write(&command)
        .map_err(|error| format!("Could not send the LG USB input command: {error}"))?;
    if !is_accepted_report_size(command_size) {
        return Err(format!(
            "LG USB input command wrote {command_size} bytes; expected {REPORT_SIZE} or {WINDOWS_REPORT_SIZE_WITH_ID}"
        ));
    }

    let (response, response_bytes_written, response_error) =
        read_response(&device, RESPONSE_READ_SIZE);

    Ok(UsbSwitchResponse {
        bytes_written: command_size + response_bytes_written,
        response,
        response_error,
    })
}

fn is_accepted_report_size(size: usize) -> bool {
    size == REPORT_SIZE || size == WINDOWS_REPORT_SIZE_WITH_ID
}

fn read_response(device: &HidDevice, expected_size: u8) -> (Vec<u8>, usize, Option<String>) {
    let mut result = Vec::new();
    let mut bytes_written = 0;
    let mut remaining = expected_size;

    while remaining > 0 {
        let chunk_size = remaining.min(MAX_RESPONSE_CHUNK_SIZE);
        thread::sleep(Duration::from_millis(10));

        let request = response_request_report(chunk_size);
        let request_size = match device.write(&request) {
            Ok(request_size) if is_accepted_report_size(request_size) => request_size,
            Ok(request_size) => {
                bytes_written += request_size;
                return (
                    result,
                    bytes_written,
                    Some(format!(
                        "LG USB response request wrote {request_size} bytes; expected {REPORT_SIZE} or {WINDOWS_REPORT_SIZE_WITH_ID}"
                    )),
                );
            }
            Err(error) => {
                return (
                    result,
                    bytes_written,
                    Some(format!(
                        "Could not request the LG USB input response: {error}"
                    )),
                );
            }
        };
        bytes_written += request_size;

        let mut response_buffer = [0_u8; REPORT_SIZE];
        let response_size = match device.read_timeout(&mut response_buffer, 800) {
            Ok(response_size) => response_size,
            Err(error) => {
                return (
                    result,
                    bytes_written,
                    Some(format!("Could not read the LG USB input response: {error}")),
                );
            }
        };
        if response_size < 4 {
            return (
                result,
                bytes_written,
                Some(format!("LG USB response was only {response_size} bytes")),
            );
        }

        let payload_size = response_buffer[0].saturating_sub(4).min(chunk_size);
        if payload_size == 0 {
            return (
                response_buffer[..response_size].to_vec(),
                bytes_written,
                Some("LG USB bridge returned no I2C response payload".into()),
            );
        }

        let payload_end = 4 + payload_size as usize;
        if payload_end > response_size {
            return (
                result,
                bytes_written,
                Some(format!(
                    "LG USB response declared {payload_size} payload bytes in a {response_size}-byte packet"
                )),
            );
        }

        result.extend_from_slice(&response_buffer[4..payload_end]);
        remaining = remaining.saturating_sub(payload_size);
    }

    (result, bytes_written, None)
}

pub(super) fn cycle_inputs(target: u8, restore: u8) -> Result<(), String> {
    send_input(target)?;
    thread::sleep(Duration::from_secs(8));
    send_input(restore)?;
    Ok(())
}

fn command_report(value: u8) -> [u8; REPORT_SIZE] {
    let ddc_frame = super::lg_ddc::lg_usb_input_set_vcp_frame(value);

    let mut report = [0_u8; REPORT_SIZE];
    report[..8].copy_from_slice(&[
        0x08,
        0x01,
        0x55,
        0x03,
        ddc_frame.len() as u8,
        0x00,
        0x03,
        super::lg_ddc::I2C_DESTINATION_ADDRESS,
    ]);
    report[8..8 + ddc_frame.len()].copy_from_slice(&ddc_frame);
    report
}

fn response_request_report(bytes_to_read: u8) -> [u8; REPORT_SIZE] {
    let mut report = [0_u8; REPORT_SIZE];
    report[..8].copy_from_slice(&[
        0x08,
        0x02,
        0x55,
        0x04,
        bytes_to_read,
        0x00,
        0x0b,
        super::lg_ddc::I2C_DESTINATION_ADDRESS,
    ]);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_lg_usb_input_report() {
        let report = command_report(0xd0);
        assert_eq!(
            &report[..15],
            &[
                0x08, 0x01, 0x55, 0x03, 0x07, 0x00, 0x03, 0x37, 0x50, 0x04, 0x03, 0xf4, 0x00, 0xd0,
                0x1d
            ]
        );
    }

    #[test]
    fn builds_lg_usb_response_request() {
        let report = response_request_report(0x26);
        assert_eq!(
            &report[..8],
            &[0x08, 0x02, 0x55, 0x04, 0x26, 0x00, 0x0b, 0x37]
        );
        assert!(report[8..].iter().all(|byte| *byte == 0));
    }
}
