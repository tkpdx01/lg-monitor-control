use std::{
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

use serialport::{ClearBuffer, SerialPort, SerialPortType};

const LG_VENDOR_ID: u16 = 0x043e;
const LG_CONTROL_PRODUCT_ID: u16 = 0x9a39;
const CDC_BAUD_RATE: u32 = 19_200;
const READ_TIMEOUT: Duration = Duration::from_millis(500);
const BYTE_DELAY: Duration = Duration::from_millis(10);
const CDC_START: [u8; 3] = [b's', b't', b'n'];
const CDC_END: [u8; 2] = [b'e', b'd'];
const DDC_SOURCE_ADDRESS: u8 = 0x51;
const DDC_CONTROL_FLAG: u8 = 0x80;
const SET_VCP_OPCODE: u8 = 0x03;
const GET_VCP_OPCODE: u8 = 0x01;
const SET_VCP_PAYLOAD_SIZE: u8 = 4;
const GET_VCP_PAYLOAD_SIZE: u8 = 2;

#[derive(Debug)]
pub(super) struct CdcResponse {
    pub port_name: String,
    pub bytes_written: usize,
    pub response: Vec<u8>,
}

pub(super) fn is_available() -> bool {
    find_control_port().is_ok()
}

pub(super) fn set_vcp(vcp_code: u8, value: u16) -> Result<CdcResponse, String> {
    send_payload(&set_vcp_payload(vcp_code, value))
}

pub(super) fn get_vcp(vcp_code: u8) -> Result<CdcResponse, String> {
    send_payload(&get_vcp_payload(vcp_code))
}

pub(super) fn send_raw(payload: &[u8]) -> Result<CdcResponse, String> {
    send_payload(payload)
}

fn find_control_port() -> Result<String, String> {
    let ports = serialport::available_ports()
        .map_err(|error| format!("Could not enumerate serial ports: {error}"))?;
    let matches = ports
        .into_iter()
        .filter(|port| match &port.port_type {
            SerialPortType::UsbPort(info) => {
                info.vid == LG_VENDOR_ID && info.pid == LG_CONTROL_PRODUCT_ID
            }
            _ => false,
        })
        .map(|port| port.port_name)
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Err(format!(
            "LG CDC control port {:04X}:{:04X} is not connected",
            LG_VENDOR_ID, LG_CONTROL_PRODUCT_ID
        )),
        [port] => Ok(port.clone()),
        ports => Err(format!(
            "{} LG CDC control ports are connected; refusing to guess the target",
            ports.len()
        )),
    }
}

fn open_control_port(port_name: &str) -> Result<Box<dyn SerialPort>, String> {
    let mut port = serialport::new(port_name, CDC_BAUD_RATE)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|error| format!("Could not open LG CDC control port {port_name}: {error}"))?;
    let _ = port.write_data_terminal_ready(false);
    let _ = port.write_request_to_send(false);
    Ok(port)
}

fn send_payload(payload: &[u8]) -> Result<CdcResponse, String> {
    let port_name = find_control_port()?;
    let mut port = open_control_port(&port_name)?;
    let mut bytes_written = 0;
    let _ = port.clear(ClearBuffer::All);

    let mut frame = Vec::with_capacity(CDC_START.len() + payload.len() + CDC_END.len());
    frame.extend_from_slice(&CDC_START);
    frame.extend_from_slice(payload);
    frame.extend_from_slice(&CDC_END);

    for byte in &frame {
        port.write_all(&[*byte]).map_err(|error| {
            format!("Could not write to LG CDC control port {port_name}: {error}")
        })?;
        bytes_written += 1;
        thread::sleep(BYTE_DELAY);
    }
    port.flush()
        .map_err(|error| format!("Could not flush LG CDC control port {port_name}: {error}"))?;

    Ok(CdcResponse {
        port_name,
        bytes_written,
        response: read_available(&mut *port)?,
    })
}

fn read_available(port: &mut dyn SerialPort) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 256];

    while Instant::now() < deadline {
        match port.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                response.extend_from_slice(&buffer[..size]);
                thread::sleep(Duration::from_millis(80));
                if port.bytes_to_read().unwrap_or_default() == 0 {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::TimedOut => break,
            Err(error) => return Err(format!("Could not read LG CDC response: {error}")),
        }
    }

    Ok(response)
}

fn set_vcp_payload(vcp_code: u8, value: u16) -> [u8; 7] {
    let high = (value >> 8) as u8;
    let low = value as u8;
    let length = DDC_CONTROL_FLAG | SET_VCP_PAYLOAD_SIZE;
    let checksum = ddc_checksum(&[
        DDC_SOURCE_ADDRESS,
        length,
        SET_VCP_OPCODE,
        vcp_code,
        high,
        low,
    ]);

    [
        DDC_SOURCE_ADDRESS,
        length,
        SET_VCP_OPCODE,
        vcp_code,
        high,
        low,
        checksum,
    ]
}

fn get_vcp_payload(vcp_code: u8) -> [u8; 5] {
    let length = DDC_CONTROL_FLAG | GET_VCP_PAYLOAD_SIZE;
    let checksum = ddc_checksum(&[DDC_SOURCE_ADDRESS, length, GET_VCP_OPCODE, vcp_code]);

    [
        DDC_SOURCE_ADDRESS,
        length,
        GET_VCP_OPCODE,
        vcp_code,
        checksum,
    ]
}

fn ddc_checksum(bytes: &[u8]) -> u8 {
    bytes
        .iter()
        .fold(super::lg_ddc::DDC_WRITE_ADDRESS, |checksum, byte| {
            checksum ^ byte
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_official_cdc_set_vcp_payload() {
        assert_eq!(
            set_vcp_payload(super::super::INPUT_SOURCE_VCP_CODE, 0x12),
            [0x51, 0x84, 0x03, 0x60, 0x00, 0x12, 0xca]
        );
        assert_eq!(
            set_vcp_payload(super::super::lg_ddc::LG_INPUT_VCP_CODE, 0x91),
            [0x51, 0x84, 0x03, 0xf4, 0x00, 0x91, 0xdd]
        );
    }

    #[test]
    fn builds_official_cdc_get_vcp_payload() {
        assert_eq!(
            get_vcp_payload(super::super::INPUT_SOURCE_VCP_CODE),
            [0x51, 0x82, 0x01, 0x60, 0xdc]
        );
    }
}
