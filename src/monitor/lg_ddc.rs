pub(super) const DDC_WRITE_ADDRESS: u8 = 0x6e;
pub(super) const I2C_DESTINATION_ADDRESS: u8 = DDC_WRITE_ADDRESS >> 1;
pub(super) const LG_SOURCE_ADDRESS: u8 = 0x50;
pub(super) const LG_INPUT_VCP_CODE: u8 = 0xf4;

const DDC_CONTROL_FLAG: u8 = 0x80;
const SET_VCP_OPCODE: u8 = 0x03;
const SET_VCP_PAYLOAD_SIZE: u8 = 4;

pub(super) fn lg_input_set_vcp_frame(value: u8) -> [u8; 7] {
    lg_source_set_vcp_frame(LG_INPUT_VCP_CODE, value as u16)
}

pub(super) fn lg_usb_input_set_vcp_frame(value: u8) -> [u8; 7] {
    lg_source_set_vcp_frame_with_length(LG_INPUT_VCP_CODE, value as u16, SET_VCP_PAYLOAD_SIZE)
}

pub(super) fn lg_source_set_vcp_frame(vcp_code: u8, value: u16) -> [u8; 7] {
    lg_source_set_vcp_frame_with_length(vcp_code, value, DDC_CONTROL_FLAG | SET_VCP_PAYLOAD_SIZE)
}

fn lg_source_set_vcp_frame_with_length(vcp_code: u8, value: u16, length: u8) -> [u8; 7] {
    let high = (value >> 8) as u8;
    let low = value as u8;
    let checksum = [
        DDC_WRITE_ADDRESS,
        LG_SOURCE_ADDRESS,
        length,
        SET_VCP_OPCODE,
        vcp_code,
        high,
        low,
    ]
    .into_iter()
    .fold(0, |checksum, byte| checksum ^ byte);

    [
        LG_SOURCE_ADDRESS,
        length,
        SET_VCP_OPCODE,
        vcp_code,
        high,
        low,
        checksum,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_lg_input_set_vcp_frame() {
        assert_eq!(
            lg_input_set_vcp_frame(0xd0),
            [0x50, 0x84, 0x03, 0xf4, 0x00, 0xd0, 0x9d]
        );
        assert_eq!(
            lg_input_set_vcp_frame(0x90),
            [0x50, 0x84, 0x03, 0xf4, 0x00, 0x90, 0xdd]
        );
    }

    #[test]
    fn encodes_lg_source_standard_input_set_vcp_frame() {
        assert_eq!(
            lg_source_set_vcp_frame(super::super::INPUT_SOURCE_VCP_CODE, 0x12),
            [0x50, 0x84, 0x03, 0x60, 0x00, 0x12, 0xcb]
        );
    }

    #[test]
    fn encodes_lg_usb_bridge_input_set_vcp_frame() {
        assert_eq!(
            lg_usb_input_set_vcp_frame(0xd0),
            [0x50, 0x04, 0x03, 0xf4, 0x00, 0xd0, 0x1d]
        );
        assert_eq!(
            lg_usb_input_set_vcp_frame(0x91),
            [0x50, 0x04, 0x03, 0xf4, 0x00, 0x91, 0x5c]
        );
    }

    #[test]
    fn encodes_igcl_full_frame_payload() {
        assert_eq!(
            lg_input_set_vcp_frame(0x91),
            [0x50, 0x84, 0x03, 0xf4, 0x00, 0x91, 0xdc]
        );
    }
}
