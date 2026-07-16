# LG WK95U Input Switching Notes

This note records the hardware-specific findings behind the LG WK95U/BK95U
fallback path. The short version: standard Windows DDC/CI can read VCP `0x60`,
but this model needs LG's private source address `0x50` and VCP `0xF4` to switch
inputs reliably.

## Confirmed Hardware

- Monitor family: LG WK95U/BK95U
- Standard input VCP: `0x60`
- Advertised standard values:
  - DisplayPort: `0x0F`
  - Thunderbolt / USB-C: `0x10`
  - HDMI 1: `0x11`
  - HDMI 2: `0x12`
- LG private values:
  - DisplayPort: `0xD0`
  - Thunderbolt / USB-C: `0xD2`
  - HDMI 1: `0x90`
  - HDMI 2: `0x91`

## CDC / Official LG USB Serial Path

LG OnScreen Control uses a CDC serial interface for many DDC/CI operations.

- USB device: `VID_043E&PID_9A39`
- Serial port observed locally: `COM3`
- Baud rate: `19200`
- Framing: `73 74 6E` (`stn`) + DDC/CI payload + `65 64` (`ed`)
- Byte pacing: approximately 10 ms between bytes

Confirmed read frame:

```text
Get VCP 0x60 payload: 51 82 01 60 DC
Response:             6E 88 02 00 60 00 00 12 00 0F C9
```

The CDC path can read the current standard VCP state. Standard SetVCP and
`0x51/0xF4` commands can return an ACK, but on this WK95U they do not apply the
input switch. Treat CDC ACK as transport-level success only.

## LG Private DDC/CI Path

The working HDMI 2 command is:

```text
50 84 03 F4 00 91 DC
```

Checksum is XOR over the DDC write address and the DDC/CI frame:

```text
6E 50 84 03 F4 00 91 = DC
```

This path is write-only in practice. After a successful private switch, standard
VCP `0x60` can still report the old input, so the app must not use that value as
authoritative for WK95U/BK95U.

## Intel IGCL Behavior

The successful Intel Graphics Control Library write uses:

```text
Address: 0x6E
Offset:  0x00
Flags:   0x00000000
Data:    50 84 03 F4 00 91 DC
```

The earlier form below returns IGCL success but does not switch this monitor:

```text
Address: 0x6E
Offset:  0x50
Data:    84 03 F4 00 91 DC
```

In other words, keep the LG private source address `0x50` inside the data frame
and leave `Offset` at zero.

## Other Paths Tested

- Windows `SetVCPFeature(0x60, 0x12)`: accepted by the API but not applied.
- Intel IGCL standard VCP `0x60`: accepted by IGCL but not applied.
- Intel IGCL `0xF4` with source address in `Offset`: accepted by IGCL but not applied.
- NVIDIA NVAPI raw I2C: NVAPI initialized, but the external display was not
  exposed as a connected NVIDIA output on the tested laptop.
- LG USB HID bridge: may write a report, but an empty I2C response payload should
  not be treated as proof of a switch. The app falls through to Intel IGCL.

## App Policy

- Use normal Windows DDC/CI first for all monitors.
- For WK95U/BK95U, fall back to LG private `0x50/0xF4` values when standard
  switching does not verify.
- Do not block a switch merely because reported VCP `0x60` equals the requested
  input on WK95U/BK95U; the reported value can be stale.
- Report LG private switching as command sent, not verified.
