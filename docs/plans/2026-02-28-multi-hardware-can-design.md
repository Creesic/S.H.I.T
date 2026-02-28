# Multi-Hardware CAN Support Design

**Date:** 2026-02-28
**Status:** Approved

## Overview

Add support for multiple CAN hardware interface types beyond the existing SLCAN serial adapters:
- **SocketCAN** (Linux native)
- **J2534/PassthruCAN** (Pass-Thru API)
- **PeakCAN/PCAN-USB** (PEAK-System devices)

## Architecture

### InterfaceType Enum Expansion

Add new interface types to `src/hardware/can_interface.rs`:

```rust
pub enum InterfaceType {
    Serial,      // Existing: SLCAN/Lawicel USB-CAN
    SocketCan,   // Linux native
    J2534,       // Pass-Thru API (Windows DLL / Linux driver)
    PeakCan,     // PEAK-System PCAN-USB
    Virtual,     // Mock for testing
    Unknown,
}
```

### Feature Flags (Cargo.toml)

```toml
[target.'cfg(target_os = "linux")'.dependencies]
socketcan = { version = "3", optional = true, features = ["tokio"] }

[target.'cfg(target_os = "windows")'.dependencies]
j2534 = { version = "0.3", optional = true }

[features]
default = []
socketcan = ["dep:socketcan"]
j2534 = ["dep:j2534"]
peakcan = []  # Uses host-can or custom FFI
all-hardware = ["socketcan", "j2534", "peakcan"]
```

### Module Structure

Each interface implements `CanInterface` trait:
- `src/hardware/j2534.rs` - J2534Interface
- `src/hardware/socketcan.rs` - SocketCanInterface
- `src/hardware/peakcan.rs` - PeakCanInterface

## J2534 Implementation

### Dependencies

- `j2534` crate (v0.3.1) - SAE J2534 PassThru bindings
- Windows: Loads vendor DLL dynamically
- Linux: Requires j2534-driver kernel module

### J2534Interface Struct

```rust
pub struct J2534Interface {
    name: String,
    status: CanStatus,
    interface: Option<j2534::Interface>,
    device: Option<j2534::Device>,
    channel: Option<j2534::Channel>,
    rx_buffer: VecDeque<CanMessage>,
    rx_count: Arc<AtomicUsize>,
    bus_id: u8,
    config: Option<CanConfig>,
}
```

### Key Operations

1. **Device Discovery**: Use `j2534::drivers()` to list installed Pass-Thru drivers
2. **Connection Flow**:
   - Load DLL: `Interface::new(path)`
   - Open device: `interface.open_any()`
   - Create channel: `device.connect(Protocol::CAN, flags, bitrate)`
3. **Message Handling**:
   - TX: `CanMessage` → `PassThruMsg::new_can(id, data)`
   - RX: Poll `channel.read()`, convert to `CanMessage`

### ISO-TP Support

For ISO15765 mode, use `Protocol::ISO15765` with `TxFlags::ISO15765_FRAME_PAD`.

### Platform Notes

- Windows: Requires 32-bit compilation per J2534 spec
- Linux: Requires j2534-driver or similar

## SocketCAN Implementation (Linux)

### Dependencies

- `socketcan` crate (v3.x) with `tokio` feature for async support

### SocketCanInterface Struct

```rust
pub struct SocketCanInterface {
    name: String,  // e.g., "can0", "vcan0"
    status: CanStatus,
    socket: Option<socketcan::tokio::CanSocket>,
    rx_buffer: VecDeque<CanMessage>,
    rx_count: Arc<AtomicUsize>,
    bus_id: u8,
    config: Option<CanConfig>,
}
```

### Key Operations

1. **Interface Discovery**: Read `/sys/class/net/` for interfaces starting with "can" or "vcan"
2. **Connection Flow**:
   - Open socket: `CanSocket::open(&name)`
   - Set bitrate via `ioctl` (physical CAN only)
3. **Message Handling**:
   - TX: `CanMessage` → `socketcan::CanFrame`
   - RX: Async read, `CanFrame` → `CanMessage`

### Virtual CAN Support

- `vcan0` for testing without hardware
- No bitrate configuration needed

## PeakCAN Implementation

### Dependencies

- `host-can` crate - Cross-platform support:
  - macOS: PCBUSB library
  - Windows: PCAN-Basic
  - Linux: Falls back to SocketCAN

### PeakCanInterface Struct

```rust
pub struct PeakCanInterface {
    name: String,  // e.g., "PCAN_USBBUS1"
    status: CanStatus,
    handle: Option<host_can::Handle>,
    rx_buffer: VecDeque<CanMessage>,
    rx_count: Arc<AtomicUsize>,
    bus_id: u8,
    config: Option<CanConfig>,
}
```

### Device Discovery

- Windows: Enumerate `PCAN_USBBUS1` through `PCAN_USBBUS16`
- macOS: PCBUSB enumeration

## UI Integration

### Interface Discovery (LiveModeState)

Update `refresh_interfaces()` with feature-gated discovery:

```rust
#[cfg(all(target_os = "linux", feature = "socketcan"))]
let socketcan = SocketCanInterface::list_interfaces();

#[cfg(all(target_os = "windows", feature = "j2534"))]
let j2534 = J2534Interface::list_drivers();

#[cfg(feature = "peakcan")]
let peakcan = PeakCanInterface::list_devices();
```

### UI Icons

| Type | Icon |
|------|------|
| Serial | `[USB]` |
| SocketCAN | `[SOC]` |
| J2534 | `[J25]` |
| PeakCAN | `[PCN]` |
| Virtual | `[SIM]` |

### CanManager Updates

Add feature-gated connection handlers in `connect_with_bus()`.

## Implementation Order

1. **SocketCAN** - Simplest, well-tested, vcan for testing
2. **J2534** - Highest priority, uses j2534 crate
3. **PeakCAN** - Cross-platform via host-can

## Testing Strategy

- **SocketCAN**: Use `vcan0` virtual interface
- **J2534**: Mock interface or actual device
- **PeakCAN**: PCAN-USB hardware or mock
