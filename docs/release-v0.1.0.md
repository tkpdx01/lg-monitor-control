# LG Monitor Control v0.1.0

First Windows release focused on reliable LG WK95U/BK95U input switching.

## Highlights

- Windows tray app for DDC/CI monitor input switching.
- Global shortcuts: `Ctrl+Alt+M` opens the app, `Ctrl+Alt+1` through
  `Ctrl+Alt+4` switch configured inputs.
- Monitor-specific input presets, aliases, favorite inputs, and compact mode.
- LG WK95U/BK95U fallback using LG private `0x50/0xF4` input commands.
- Intel IGCL raw I2C support for the WK95U/BK95U private path.
- USB CDC diagnostics for reading VCP state and validating LG transport behavior.

## LG WK95U/BK95U Fix

The release includes the verified Intel IGCL encoding for HDMI 2:

```text
address=0x6E offset=0 flags=0 data=50 84 03 F4 00 91 DC
```

The app no longer treats WK95U/BK95U VCP `0x60` current-input reads as
authoritative after private switching, because the monitor can continue
reporting the old input even after the physical input changes.

## Requirements

- Windows 10 or later.
- DDC/CI enabled in the monitor OSD.
- For the LG private fallback on Intel graphics: Intel driver with
  `C:\Windows\System32\ControlLib.dll`.
- Administrator approval may be requested only when the Intel IGCL fallback is
  used.

## Artifact

- `lg-monitor-control-v0.1.0-windows-x64.zip`
