//! rusEFI Wideband CAN Plugin
//!
//! Implements the rusEFI wideband protocol matching TunerStudio's "rusEFI Wideband Tools".
//! See rusefi_wideband_can_protocol.md and epicefi_fw firmware.
//!
//! Protocol summary:
//! - Sensor data: 11-bit IDs 0x190 + (2×index) for standard, +1 for diagnostic
//! - ECU status (heater enable): Extended ID 0x0EF50000, send every 10ms
//! - Commands: Ping, Set Index, Set Sensor Type, Restart
//! - Firmware update: 0xEF bootloader protocol (Enter, Erase, Write, Reboot)
//! - 500 kbps, little-endian

use crate::core::CanMessage;
use crate::plugins::Plugin;
use crate::ui::FileDialogs;
use imgui::{Condition, Ui};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

// Protocol constants (from rusefi_wideband_can_protocol.md and rusefi_wideband.cpp)
const WB_ACK_ID: u32 = 0x727573;
const WB_DATA_BASE: u32 = 0x190;
const WB_CMD_ECU_STATUS: u32 = 0x0EF50000;
const WB_CMD_SET_INDEX: u32 = 0x0EF40000;
const WB_CMD_PING: u32 = 0x0EF60000;
const WB_CMD_SET_SENSOR_TYPE: u32 = 0x0EF70000;

// Bootloader commands (0xEF protocol, F0 module / rusEFI beam)
const WB_BL_ENTER: u32 = 0x0EF00000;
const WB_BL_ERASE: u32 = 0x0EF15A5A;
const WB_BL_DATA_BASE: u32 = 0x0EF20000;
const WB_BL_REBOOT: u32 = 0x0EF30000;

const STATUS_OK: u8 = 0;
const STATUS_HEATER_FAULT: u8 = 1;
const STATUS_SENSOR_ERROR: u8 = 2;

/// Command status (matches canReWidebandCmdStatus)
#[derive(Clone, Copy, PartialEq)]
enum CmdStatus {
    Idle = 0,
    Done = 1,
    Busy = 2,
    Failed = 3,
}

/// Firmware flash state machine (0xEF bootloader protocol)
#[derive(Clone, Copy, PartialEq, Debug)]
enum FlashState {
    Idle,
    EnterBl,
    WaitingEnterAck,
    /// Delay after Enter ACK - bootloader needs time to initialize
    EraseDelay,
    Erase,
    WaitingEraseAck,
    Writing,
    WaitingWriteAck,
    Reboot,
    WaitingRebootAck,
    Done,
    Failed,
}

/// Sensor types (matches canReWidebandSensorType)
const SENSOR_NAMES: [&str; 4] = [
    "Bosch LSU 4.9",
    "Bosch LSU 4.2",
    "Bosch LSU ADV",
    "FAE LSU 4.9",
];

fn status_name(s: u8) -> &'static str {
    match s {
        STATUS_OK => "OK",
        STATUS_HEATER_FAULT => "HEATER_FAULT",
        STATUS_SENSOR_ERROR => "SENSOR_ERROR",
        _ => "UNKNOWN",
    }
}

/// Find BootCommander executable. Checks: same dir as S.H.I.T exe, ./dist, PATH.
fn find_bootcommander() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "BootCommander.exe"
    } else {
        "BootCommander"
    };
    // 1. Next to the S.H.I.T executable (bundled)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // 2. ./dist (project layout when built with build-bootcommander.sh)
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("dist").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // 3. PATH
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

/// Run BootCommander for F1/XCP flash. Returns (success, output).
fn run_bootcommander(path: &std::path::Path, interface: &str) -> (bool, String) {
    let exe_path = match find_bootcommander() {
        Some(p) => p,
        None => {
            let name = if cfg!(windows) { "BootCommander.exe" } else { "BootCommander" };
            return (
                false,
                format!(
                    "BootCommander not found. Run ./scripts/build-bootcommander.sh and add dist/ to PATH, \
                     or place {} next to the S.H.I.T executable.",
                    name
                ),
            );
        }
    };
    let iface_lower = interface.to_lowercase();
    let (transport, device) = if iface_lower.contains("can") || iface_lower.starts_with("slcan")
        || iface_lower == "can0"
        || iface_lower == "can1"
    {
        ("xcp_can", interface.to_string())
    } else if iface_lower.contains("tty") || iface_lower.starts_with("com") {
        ("xcp_rs232", interface.to_string())
    } else {
        ("xcp_can", interface.to_string())
    };

    let mut cmd = Command::new(&exe_path);
    cmd.arg(format!("-t={}", transport))
        .arg(format!("-d={}", device))
        .arg("-t1=1000")
        .arg("-t3=2000")
        .arg("-t4=10000")
        .arg("-t5=1000")
        .arg("-t7=2000");

    if transport == "xcp_rs232" {
        cmd.arg("-b=115200");
    }

    cmd.arg(path);

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else if stdout.is_empty() {
                stderr
            } else {
                format!("{}{}", stdout, stderr)
            };
            let success = output.status.success();
            (success, combined)
        }
        Err(e) => (
            false,
            format!("Failed to run BootCommander: {}.", e),
        ),
    }
}

/// F0 module layout: 6KB bootloader, 25KB app, 1KB config
const F0_BOOTLOADER_SIZE: usize = 6 * 1024;
const F0_APP_SIZE: usize = 25 * 1024;

/// Extract app region from a full flash image. If the file looks like a 32KB
/// image (bootloader + app + config), strip the bootloader and return only
/// the 25KB app. Otherwise return the data as-is.
fn extract_app_from_image(data: Vec<u8>) -> Vec<u8> {
    if data.len() >= F0_BOOTLOADER_SIZE + F0_APP_SIZE {
        data[F0_BOOTLOADER_SIZE..F0_BOOTLOADER_SIZE + F0_APP_SIZE]
            .to_vec()
    } else {
        data
    }
}

/// Parse SREC/S19 file into raw binary. Returns None on parse error.
fn parse_srec(data: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(data).ok()?;
    let mut min_addr = u32::MAX;
    let mut max_addr = 0u32;
    let mut segments: Vec<(u32, Vec<u8>)> = Vec::new();

    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('S') {
            continue;
        }
        let rest = line.get(1..)?;
        let record_type = rest.chars().next()?;
        let hex = rest.get(1..)?;
        if hex.len() % 2 != 0 {
            return None;
        }
        let bytes: Result<Vec<u8>, _> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect();
        let bytes = bytes.ok()?;
        if bytes.len() < 3 {
            continue;
        }
        let len = bytes[0] as usize;
        if bytes.len() < len + 1 {
            continue;
        }
        let checksum_idx = len + 1;
        let payload = &bytes[1..len];
        let _checksum = bytes.get(checksum_idx)?;

        match record_type {
            '1' => {
                if payload.len() < 2 {
                    continue;
                }
                let addr = (payload[0] as u32) << 8 | (payload[1] as u32);
                let data = payload[2..].to_vec();
                min_addr = min_addr.min(addr);
                max_addr = max_addr.max(addr + data.len() as u32 - 1);
                segments.push((addr, data));
            }
            '2' => {
                if payload.len() < 3 {
                    continue;
                }
                let addr = (payload[0] as u32) << 16 | (payload[1] as u32) << 8 | (payload[2] as u32);
                let data = payload[3..].to_vec();
                min_addr = min_addr.min(addr);
                max_addr = max_addr.max(addr + data.len() as u32 - 1);
                segments.push((addr, data));
            }
            '3' => {
                if payload.len() < 4 {
                    continue;
                }
                let addr = (payload[0] as u32) << 24
                    | (payload[1] as u32) << 16
                    | (payload[2] as u32) << 8
                    | (payload[3] as u32);
                let data = payload[4..].to_vec();
                min_addr = min_addr.min(addr);
                max_addr = max_addr.max(addr + data.len() as u32 - 1);
                segments.push((addr, data));
            }
            '0' | '5' | '7' | '8' | '9' => {}
            _ => {}
        }
    }

    if min_addr == u32::MAX || segments.is_empty() {
        return None;
    }

    let size = (max_addr - min_addr + 1) as usize;
    if size > 26 * 1024 {
        return None;
    }
    let mut result = vec![0xFF; size];
    for (addr, data) in segments {
        let offset = (addr - min_addr) as usize;
        for (i, &b) in data.iter().enumerate() {
            if offset + i < result.len() {
                result[offset + i] = b;
            }
        }
    }
    Some(result)
}

/// Pong response data (from WB_ACK_ID with DLC=8)
#[derive(Clone, Debug, Default)]
struct PongData {
    version: u8,
    day: u8,
    month: u8,
    year: u8,
}

/// Standard sensor data (lambda, temperature)
#[derive(Clone, Debug, Default)]
struct WidebandData {
    index: u8,
    valid: bool,
    lambda_value: f32,
    afr_gasoline: f32,
    temp_c: i16,
}

/// Diagnostic data (ESR, Nernst, status)
#[derive(Clone, Debug, Default)]
struct WidebandDiagnostics {
    index: u8,
    esr_ohms: u16,
    nernst_v: f32,
    pump_duty_pct: f32,
    heater_duty_pct: f32,
    status: u8,
}

pub struct RusefiWidebandPlugin {
    sensor_data: HashMap<u8, WidebandData>,
    diagnostics: HashMap<u8, WidebandDiagnostics>,
    /// Pong response (FW version, build date)
    pong: Option<PongData>,
    /// Heater enabled (sends ECU status periodically)
    heater_enabled: bool,
    /// Battery voltage for ECU status (×10, e.g. 135 = 13.5V)
    voltage_x10: u8,
    /// Bus to use for TX
    tx_bus: u8,
    /// Last ECU status send time
    last_ecu_status: Option<Instant>,
    /// ECU status interval (10ms per protocol)
    ecu_status_interval: Duration,
    /// Restart: stop heater for 0.5s (10 × 50ms)
    restart_remaining: u8,
    /// Target device HW index (0-7 = specific, 0xff = broadcast)
    hw_index: u8,
    /// Required CAN ID index (0-15) for Set Index
    can_id_index: u8,
    /// Sensor type for Set Sensor Type (0-3)
    sensor_type: u8,
    /// Command status
    cmd_status: CmdStatus,
    /// When we last sent a command (for timeout)
    cmd_sent_at: Option<Instant>,
    /// Firmware flash state
    flash_state: FlashState,
    /// Firmware image to flash (raw .bin)
    flash_data: Option<Vec<u8>>,
    /// Current write offset (bytes)
    flash_offset: usize,
    /// When we last sent a flash command (for timeout)
    flash_sent_at: Option<Instant>,
    /// When we first sent Enter (for 5s timeout; retransmit uses different interval)
    flash_enter_started_at: Option<Instant>,
    /// Error message for flash failure
    flash_error: Option<String>,
    /// Skip ACK wait - use fixed delays instead (for adapters that don't pass ACK through)
    flash_skip_ack: bool,
    /// BootCommander (F1/XCP) flash: interface override (e.g. "can0" or "peak_pcanusb")
    bootcommander_interface: String,
    /// BootCommander: running or last result
    bootcommander_state: BootCommanderState,
    /// Result from BootCommander thread (success, output)
    bootcommander_result: Arc<Mutex<Option<(bool, String)>>>,
}

#[derive(Clone, Default)]
enum BootCommanderState {
    #[default]
    Idle,
    Running,
    Done { success: bool, output: String },
}

impl RusefiWidebandPlugin {
    pub fn new() -> Self {
        Self {
            sensor_data: HashMap::new(),
            diagnostics: HashMap::new(),
            pong: None,
            heater_enabled: false,
            voltage_x10: 135, // 13.5V
            tx_bus: 0,
            last_ecu_status: None,
            ecu_status_interval: Duration::from_millis(10),
            restart_remaining: 0,
            hw_index: 0xff, // Broadcast by default
            can_id_index: 0,
            sensor_type: 0, // LSU 4.9
            cmd_status: CmdStatus::Idle,
            cmd_sent_at: None,
            flash_state: FlashState::Idle,
            flash_data: None,
            flash_offset: 0,
            flash_sent_at: None,
            flash_enter_started_at: None,
            flash_error: None,
            flash_skip_ack: false,
            bootcommander_interface: String::new(),
            bootcommander_state: BootCommanderState::Idle,
            bootcommander_result: Arc::new(Mutex::new(None)),
        }
    }

    fn decode_standard_frame(&self, can_id: u32, data: &[u8]) -> Option<WidebandData> {
        if data.len() != 8 {
            return None;
        }
        if can_id < WB_DATA_BASE || can_id > WB_DATA_BASE + 30 || can_id % 2 != 0 {
            return None;
        }

        let index = ((can_id - WB_DATA_BASE) / 2) as u8;
        let valid = data[1] == 1;
        let lambda_raw = u16::from_le_bytes([data[2], data[3]]);
        let temp_raw = i16::from_le_bytes([data[4], data[5]]);

        let lambda_value = lambda_raw as f32 / 10000.0;
        let afr = lambda_value * 14.7;

        Some(WidebandData {
            index,
            valid,
            lambda_value,
            afr_gasoline: afr,
            temp_c: temp_raw,
        })
    }

    fn decode_diagnostic_frame(&self, can_id: u32, data: &[u8]) -> Option<WidebandDiagnostics> {
        if data.len() != 8 {
            return None;
        }
        if can_id < WB_DATA_BASE + 1 || can_id > WB_DATA_BASE + 31 || can_id % 2 != 1 {
            return None;
        }

        let index = ((can_id - WB_DATA_BASE - 1) / 2) as u8;
        let esr = u16::from_le_bytes([data[0], data[1]]);
        let nernst_mv = u16::from_le_bytes([data[2], data[3]]) as f32 / 1000.0;
        let pump_duty = data[4] as f32 / 2.55;
        let status = data[5];
        let heater_duty = data[6] as f32 / 2.55;

        Some(WidebandDiagnostics {
            index,
            esr_ohms: esr,
            nernst_v: nernst_mv,
            pump_duty_pct: pump_duty,
            heater_duty_pct: heater_duty,
            status,
        })
    }

    fn decode_pong(&self, data: &[u8]) -> Option<PongData> {
        if data.len() != 8 {
            return None;
        }
        // Firmware uses: Version, day, month, year (wbo::PongData)
        Some(PongData {
            version: data[0],
            day: data[1],
            month: data[2],
            year: data[3],
        })
    }

    fn process_message(&mut self, msg: &crate::hardware::can_manager::ManagerMessage) {
        let can_id = msg.message.id;
        let data = &msg.message.data;

        if (WB_DATA_BASE..=WB_DATA_BASE + 31).contains(&can_id) {
            if can_id % 2 == 0 {
                if let Some(d) = self.decode_standard_frame(can_id, data) {
                    self.sensor_data.insert(d.index, d);
                }
            } else if let Some(d) = self.decode_diagnostic_frame(can_id, data) {
                self.diagnostics.insert(d.index, d);
            }
        } else if can_id == WB_ACK_ID {
            if self.flash_state != FlashState::Idle {
                info!("Flash: received ACK (0x{:08X}) in state {:?}", can_id, self.flash_state);
                // Bootloader ACK - advance flash state machine
                self.flash_sent_at = None;
                match self.flash_state {
                    FlashState::WaitingEnterAck => {
                        self.flash_state = FlashState::EraseDelay;
                        self.flash_sent_at = Some(Instant::now());
                        self.flash_enter_started_at = None;
                    }
                    FlashState::WaitingEraseAck => self.flash_state = FlashState::Writing,
                    FlashState::WaitingWriteAck => {
                        self.flash_offset += 8;
                        if self.flash_offset >= self.flash_data.as_ref().map_or(0, Vec::len) {
                            self.flash_state = FlashState::Reboot;
                        } else {
                            self.flash_state = FlashState::Writing;
                        }
                    }
                    FlashState::WaitingRebootAck => self.flash_state = FlashState::Done,
                    _ => {}
                }
            } else {
                if data.len() == 8 {
                    if let Some(pong) = self.decode_pong(data) {
                        self.pong = Some(pong);
                    }
                }
                // ACK (DLC=0) or Pong (DLC=8) - both indicate command completed
                self.cmd_status = CmdStatus::Done;
                self.cmd_sent_at = None;
            }
        }
    }

    /// Build ECU status message for heater enable
    fn build_ecu_status(&self) -> CanMessage {
        let flags = if self.heater_enabled && self.restart_remaining == 0 {
            0x01u8
        } else {
            0x00
        };
        let data = vec![self.voltage_x10, flags];
        CanMessage::new(self.tx_bus, WB_CMD_ECU_STATUS, data)
    }

    fn queue_ping(&self, ctx: &mut crate::plugins::PluginContext) {
        let data = vec![self.hw_index];
        ctx.queue_send.push((
            self.tx_bus,
            CanMessage::new(self.tx_bus, WB_CMD_PING, data),
        ));
    }

    fn queue_set_index(&self, ctx: &mut crate::plugins::PluginContext) {
        let data = if self.hw_index == 0xff {
            vec![self.can_id_index]
        } else {
            vec![self.can_id_index, self.hw_index]
        };
        let msg = CanMessage::new(self.tx_bus, WB_CMD_SET_INDEX, data);
        // Spam 100x in case controller misses some (bus timing, filtering, etc.)
        for _ in 0..100 {
            ctx.queue_send.push((self.tx_bus, msg.clone()));
        }
    }

    fn queue_set_sensor_type(&self, ctx: &mut crate::plugins::PluginContext) {
        let data = vec![self.hw_index, self.sensor_type];
        ctx.queue_send.push((
            self.tx_bus,
            CanMessage::new(self.tx_bus, WB_CMD_SET_SENSOR_TYPE, data),
        ));
    }

    fn queue_flash_enter(&self, ctx: &mut crate::plugins::PluginContext) {
        let data = if self.hw_index == 0xff {
            vec![]
        } else {
            vec![self.hw_index]
        };
        let msg = CanMessage::new(self.tx_bus, WB_BL_ENTER, data);
        for _ in 0..5 {
            ctx.queue_send.push((self.tx_bus, msg.clone()));
        }
    }

    fn queue_flash_erase(&self, ctx: &mut crate::plugins::PluginContext) {
        ctx.queue_send.push((
            self.tx_bus,
            CanMessage::new(self.tx_bus, WB_BL_ERASE, vec![]),
        ));
    }

    fn queue_flash_write(&self, ctx: &mut crate::plugins::PluginContext, offset: u16, chunk: [u8; 8]) {
        let can_id = WB_BL_DATA_BASE | (offset as u32);
        ctx.queue_send.push((
            self.tx_bus,
            CanMessage::new(self.tx_bus, can_id, chunk.to_vec()),
        ));
    }

    fn queue_flash_reboot(&self, ctx: &mut crate::plugins::PluginContext) {
        ctx.queue_send.push((
            self.tx_bus,
            CanMessage::new(self.tx_bus, WB_BL_REBOOT, vec![]),
        ));
    }
}

impl Plugin for RusefiWidebandPlugin {
    fn id(&self) -> &str {
        "rusefi_wideband"
    }

    fn name(&self) -> &str {
        "rusEFI Wideband"
    }

    fn description(&self) -> &str {
        "rusEFI Wideband Tools - lambda/AFR, Ping, Set Index, Set Sensor Type, Restart"
    }

    fn render(
        &mut self,
        ui: &Ui,
        ctx: &mut crate::plugins::PluginContext,
        messages: &[crate::hardware::can_manager::ManagerMessage],
        is_open: &mut bool,
    ) {
        // Process incoming messages
        for msg in messages {
            self.process_message(msg);
        }

        // Check BootCommander result
        if matches!(self.bootcommander_state, BootCommanderState::Running) {
            if let Ok(mut guard) = self.bootcommander_result.lock() {
                if let Some((success, output)) = guard.take() {
                    self.bootcommander_state = BootCommanderState::Done { success, output };
                }
            }
        }

        // Restart countdown
        if self.restart_remaining > 0 {
            self.restart_remaining = self.restart_remaining.saturating_sub(1);
        }

        // Flash state machine: send next command when in "send" state
        if ctx.is_connected && self.flash_state != FlashState::Idle && self.flash_state != FlashState::Done && self.flash_state != FlashState::Failed {
            let now = Instant::now();
            match self.flash_state {
                FlashState::EnterBl => {
                    self.queue_flash_enter(ctx);
                    self.flash_state = FlashState::WaitingEnterAck;
                    self.flash_sent_at = Some(now);
                    self.flash_enter_started_at = Some(now);
                }
                FlashState::WaitingEnterAck => {
                    if self.flash_skip_ack {
                        // Skip ACK: advance after 1.5s
                        if self
                            .flash_sent_at
                            .map_or(false, |t| now.duration_since(t) > Duration::from_millis(1500))
                        {
                            self.flash_state = FlashState::EraseDelay;
                            self.flash_sent_at = Some(now);
                            self.flash_enter_started_at = None;
                        }
                    } else {
                        // Timeout after 5s (bootloader may need several Enter retries after reboot)
                        if self.flash_enter_started_at.map_or(false, |t| {
                            now.duration_since(t) > Duration::from_secs(5)
                        }) {
                            self.flash_state = FlashState::Failed;
                            self.flash_error =
                                Some("Timeout waiting for bootloader ACK".to_string());
                            self.flash_sent_at = None;
                            self.flash_enter_started_at = None;
                        } else if self
                            .flash_sent_at
                            .map_or(true, |t| now.duration_since(t) > Duration::from_millis(150))
                        {
                            // Resend Enter periodically - bootloader waits for it after app reboots
                            self.queue_flash_enter(ctx);
                            self.flash_sent_at = Some(now);
                        }
                    }
                }
                FlashState::EraseDelay => {
                    // Bootloader needs ~500-800ms to start after reboot
                    if self
                        .flash_sent_at
                        .map_or(false, |t| now.duration_since(t) > Duration::from_millis(800))
                    {
                        self.flash_state = FlashState::Erase;
                        self.flash_sent_at = None;
                    }
                }
                FlashState::Erase => {
                    self.queue_flash_erase(ctx);
                    self.flash_state = FlashState::WaitingEraseAck;
                    self.flash_sent_at = Some(now);
                }
                FlashState::Writing => {
                    if let Some(ref data) = self.flash_data {
                        if self.flash_offset + 8 <= data.len() {
                            let mut chunk = [0u8; 8];
                            chunk.copy_from_slice(&data[self.flash_offset..self.flash_offset + 8]);
                            self.queue_flash_write(ctx, self.flash_offset as u16, chunk);
                            self.flash_state = FlashState::WaitingWriteAck;
                            self.flash_sent_at = Some(now);
                        } else {
                            self.flash_state = FlashState::Failed;
                            self.flash_error = Some("Invalid flash offset".to_string());
                        }
                    } else {
                        self.flash_state = FlashState::Failed;
                        self.flash_error = Some("No firmware data".to_string());
                    }
                }
                FlashState::Reboot => {
                    self.queue_flash_reboot(ctx);
                    self.flash_state = FlashState::WaitingRebootAck;
                    self.flash_sent_at = Some(now);
                }
                FlashState::WaitingEraseAck | FlashState::WaitingWriteAck | FlashState::WaitingRebootAck => {
                    if self.flash_skip_ack {
                        // Skip ACK mode: advance after fixed delay
                        let delay = match self.flash_state {
                            FlashState::WaitingEraseAck => Duration::from_secs(2),
                            FlashState::WaitingWriteAck => Duration::from_millis(8),
                            FlashState::WaitingRebootAck => Duration::from_millis(500),
                            _ => Duration::from_secs(1),
                        };
                        if let Some(sent_at) = self.flash_sent_at {
                            if now.duration_since(sent_at) > delay {
                                self.flash_sent_at = None;
                                match self.flash_state {
                                    FlashState::WaitingEraseAck => self.flash_state = FlashState::Writing,
                                    FlashState::WaitingWriteAck => {
                                        self.flash_offset += 8;
                                        if self.flash_offset
                                            >= self.flash_data.as_ref().map_or(0, Vec::len)
                                        {
                                            self.flash_state = FlashState::Reboot;
                                        } else {
                                            self.flash_state = FlashState::Writing;
                                        }
                                    }
                                    FlashState::WaitingRebootAck => self.flash_state = FlashState::Done,
                                    _ => {}
                                }
                            }
                        }
                    } else {
                        // Normal mode: timeout = failure
                        let timeout = match self.flash_state {
                            FlashState::WaitingEraseAck => Duration::from_secs(3),
                            FlashState::WaitingWriteAck => Duration::from_millis(300),
                            FlashState::WaitingRebootAck => Duration::from_secs(3),
                            _ => Duration::from_secs(1),
                        };
                        if let Some(sent_at) = self.flash_sent_at {
                            if now.duration_since(sent_at) > timeout {
                                self.flash_state = FlashState::Failed;
                                self.flash_error = Some("Timeout waiting for ACK".to_string());
                                self.flash_sent_at = None;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Timeout Busy after 500ms with no response
        if self.cmd_status == CmdStatus::Busy {
            if let Some(sent_at) = self.cmd_sent_at {
                if sent_at.elapsed() > Duration::from_millis(500) {
                    self.cmd_status = CmdStatus::Failed;
                    self.cmd_sent_at = None;
                }
            }
        }

        // Queue ECU status periodically when heater enabled (and not in restart)
        if ctx.is_connected && self.heater_enabled {
            let now = Instant::now();
            let should_send = self
                .last_ecu_status
                .map_or(true, |t| now.duration_since(t) >= self.ecu_status_interval);
            if should_send {
                let ecu_msg = self.build_ecu_status();
                ctx.queue_send.push((self.tx_bus, ecu_msg));
                self.last_ecu_status = Some(now);
            }
        }

        ui.window("rusEFI Wideband Tools")
            .size([420.0, 520.0], Condition::FirstUseEver)
            .position([50.0, 100.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                if !ctx.is_connected && !ctx.has_playback {
                    ui.text_colored([1.0, 0.5, 0.3, 1.0], "No CAN interface connected");
                    ui.text_wrapped("Connect to a CAN interface in Hardware Manager first, or open a CAN log (CSV/rlog) for playback. The wideband controller requires ECU status messages every 10ms to enable the heater when live.");
                    return;
                }
                if ctx.has_playback && !ctx.is_connected {
                    ui.text_colored([0.5, 0.8, 0.5, 1.0], "Playback mode");
                    ui.text_wrapped("Showing wideband data from loaded log. Heater and commands require a live CAN connection.");
                    ui.separator();
                }

                if ctx.is_connected {
                ui.text("Configuration");
                ui.separator();

                ui.text("TX Bus:");
                ui.same_line();
                let bus_labels: Vec<String> =
                    ctx.connected_buses.iter().map(|b| format!("Bus {}", b)).collect();
                if !bus_labels.is_empty() {
                    let mut bus_idx = ctx
                        .connected_buses
                        .iter()
                        .position(|&b| b == self.tx_bus)
                        .unwrap_or(0);
                    if bus_idx >= bus_labels.len() {
                        bus_idx = 0;
                    }
                    let labels_ref: Vec<&str> = bus_labels.iter().map(|s| s.as_str()).collect();
                    if ui.combo_simple_string("##tx_bus", &mut bus_idx, &labels_ref) {
                        if let Some(&bus) = ctx.connected_buses.get(bus_idx) {
                            self.tx_bus = bus;
                        }
                    }
                } else {
                    ui.text("(no buses)");
                }

                ui.text("Battery voltage:");
                ui.same_line();
                let mut v = self.voltage_x10 as i32;
                if ui.input_int("##voltage", &mut v).build() {
                    self.voltage_x10 = v.clamp(80, 160) as u8; // 8.0V - 16.0V
                }
                ui.same_line();
                ui.text(format!("({:.1}V)", self.voltage_x10 as f32 / 10.0));

                ui.checkbox("Heater enabled", &mut self.heater_enabled);
                ui.same_line();
                ui.text_disabled("(?)");
                if ui.is_item_hovered() {
                    ui.tooltip(|| {
                        ui.text_wrapped("The controller requires ECU status with heater bit set to produce valid data. Warm-up takes 10-30 seconds.");
                    });
                }

                ui.separator();
                ui.text("2025 firmware tools");
                ui.separator();

                // Target device HW ID
                ui.text("Target device HW ID:");
                ui.same_line();
                let hw_labels = [
                    "Broadcast (any)",
                    "Idx 0",
                    "Idx 1",
                    "Idx 2",
                    "Idx 3",
                    "Idx 4",
                    "Idx 5",
                    "Idx 6",
                    "Idx 7",
                ];
                let mut hw_idx = if self.hw_index == 0xff {
                    0
                } else {
                    (self.hw_index as usize).min(7) + 1
                };
                if ui.combo_simple_string("##hw_index", &mut hw_idx, &hw_labels) {
                    self.hw_index = if hw_idx == 0 { 0xff } else { (hw_idx - 1) as u8 };
                }

                // Ping button
                if ui.button("Ping / Get FW version") {
                    self.cmd_status = CmdStatus::Busy;
                    self.cmd_sent_at = Some(Instant::now());
                    self.pong = None;
                    self.queue_ping(ctx);
                }
                ui.same_line();
                let status_str = match self.cmd_status {
                    CmdStatus::Idle => "Idle",
                    CmdStatus::Done => "Done",
                    CmdStatus::Busy => "Busy",
                    CmdStatus::Failed => "Failed",
                };
                let status_color = match self.cmd_status {
                    CmdStatus::Idle => [0.6, 0.6, 0.6, 1.0],
                    CmdStatus::Done => [0.3, 0.8, 0.3, 1.0],
                    CmdStatus::Busy => [0.8, 0.8, 0.3, 1.0],
                    CmdStatus::Failed => [1.0, 0.3, 0.3, 1.0],
                };
                ui.text_colored(status_color, status_str);

                // FW version and build date
                if let Some(ref p) = self.pong {
                    let year_display = 2000u32 + p.year as u32;
                    ui.text(format!(
                        "FW: v{}  Build: {}/{}/{}",
                        p.version, p.day, p.month, year_display
                    ));
                } else {
                    ui.text_colored([0.5, 0.5, 0.5, 1.0], "FW version: (ping to get)");
                }

                ui.separator();

                // Required CAN ID
                ui.text("Required CAN ID:");
                ui.same_line();
                let can_id_labels: Vec<String> = (0..16)
                    .map(|i| {
                        let base = WB_DATA_BASE + 2 * i;
                        format!("ID{} 0x{:03X}/0x{:03X}", i + 1, base, base + 1)
                    })
                    .collect();
                let can_id_refs: Vec<&str> = can_id_labels.iter().map(|s| s.as_str()).collect();
                let mut can_idx = self.can_id_index as usize;
                if ui.combo_simple_string("##can_id", &mut can_idx, &can_id_refs) {
                    self.can_id_index = can_idx as u8;
                }

                if ui.button("Set Index") {
                    self.cmd_status = CmdStatus::Busy;
                    self.cmd_sent_at = Some(Instant::now());
                    self.queue_set_index(ctx);
                }

                ui.separator();

                // Sensor type
                ui.text("Sensor type:");
                ui.same_line();
                let mut sens_idx = self.sensor_type as usize;
                if ui.combo_simple_string("##sensor_type", &mut sens_idx, &SENSOR_NAMES) {
                    self.sensor_type = sens_idx as u8;
                }

                if ui.button("Set sensor type") {
                    self.cmd_status = CmdStatus::Busy;
                    self.cmd_sent_at = Some(Instant::now());
                    self.queue_set_sensor_type(ctx);
                }

                ui.separator();

                // Restart
                if ui.button("Restart all WBO") {
                    self.restart_remaining = 50; // 0.5s at 10ms interval (matches firmware)
                }
                ui.same_line();
                ui.text_disabled("(?)");
                if ui.is_item_hovered() {
                    ui.tooltip(|| {
                        ui.text_wrapped("Stops sending heater enable for ~0.5s. Use to reset controllers.");
                    });
                }

                ui.separator();
                ui.text("Firmware update (CAN)");
                ui.separator();

                ui.checkbox("Skip ACK wait (use fixed delays)", &mut self.flash_skip_ack);
                ui.same_line();
                ui.text_disabled("(?)");
                if ui.is_item_hovered() {
                    ui.tooltip(|| {
                        ui.text_wrapped("If ACK timeout occurs, enable this to use fixed delays instead of waiting for bootloader ACK. Use when your CAN adapter doesn't pass ACK through, or for debugging.");
                    });
                }

                let can_flash = self.flash_state == FlashState::Idle || self.flash_state == FlashState::Done || self.flash_state == FlashState::Failed;
                if can_flash {
                    if ui.button("Flash firmware...") {
                        if let Some(path) = FileDialogs::open_firmware_file() {
                            match std::fs::read(&path) {
                                Ok(raw) => {
                                    let data: Option<Vec<u8>> = if path
                                        .extension()
                                        .map_or(false, |e| e == "srec" || e == "s19")
                                    {
                                        parse_srec(&raw)
                                    } else {
                                        Some(raw)
                                    };
                                    match data {
                                        None => {
                                            self.flash_error =
                                                Some("Failed to parse SREC file".to_string());
                                            self.flash_state = FlashState::Failed;
                                        }
                                        Some(data) => {
                                            let mut data = extract_app_from_image(data);
                                            if data.is_empty() {
                                                self.flash_error =
                                                    Some("File is empty".to_string());
                                                self.flash_state = FlashState::Failed;
                                            } else if data.len() > 26 * 1024 {
                                                self.flash_error = Some(format!(
                                                    "File too large ({} bytes, max 26KB)",
                                                    data.len()
                                                ));
                                                self.flash_state = FlashState::Failed;
                                            } else {
                                                let remainder = data.len() % 8;
                                                if remainder != 0 {
                                                    data.extend(
                                                        std::iter::repeat(0xFF).take(8 - remainder),
                                                    );
                                                }
                                                self.flash_data = Some(data);
                                                self.flash_offset = 0;
                                                self.flash_state = FlashState::EnterBl;
                                                self.flash_error = None;
                                                self.flash_sent_at = None;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    self.flash_error =
                                        Some(format!("Failed to read file: {}", e));
                                    self.flash_state = FlashState::Failed;
                                }
                            }
                        }
                    }
                    ui.same_line();
                    ui.text_disabled("(?)");
                    if ui.is_item_hovered() {
                        ui.tooltip(|| {
                            ui.text_wrapped("Select a .bin firmware file and flash via 0xEF bootloader protocol. Works with rusEFI F0 module / rusEFI beam compatible controllers. F1 boards use OpenBLT/XCP - use BootCommander instead.");
                        });
                    }
                } else {
                    let (label, color) = match self.flash_state {
                        FlashState::EnterBl | FlashState::WaitingEnterAck => ("Entering bootloader...".into(), [0.8, 0.8, 0.3, 1.0]),
                        FlashState::EraseDelay => ("Bootloader starting...".into(), [0.8, 0.8, 0.3, 1.0]),
                        FlashState::Erase | FlashState::WaitingEraseAck => ("Erasing flash...".into(), [0.8, 0.8, 0.3, 1.0]),
                        FlashState::Writing | FlashState::WaitingWriteAck => {
                            let total = self.flash_data.as_ref().map_or(0, Vec::len);
                            let pct = if total > 0 { (self.flash_offset * 100) / total } else { 0 };
                            (format!("Writing... {}%", pct), [0.8, 0.8, 0.3, 1.0])
                        }
                        FlashState::Reboot | FlashState::WaitingRebootAck => ("Rebooting...".into(), [0.8, 0.8, 0.3, 1.0]),
                        FlashState::Done => ("Done!".into(), [0.3, 0.8, 0.3, 1.0]),
                        FlashState::Failed => ("Failed".into(), [1.0, 0.3, 0.3, 1.0]),
                        _ => ("Flashing...".into(), [0.8, 0.8, 0.3, 1.0]),
                    };
                    ui.text_colored(color, &label);
                }
                if let Some(ref err) = self.flash_error {
                    ui.text_colored([1.0, 0.3, 0.3, 1.0], err);
                }
                if self.flash_state == FlashState::Done {
                    self.flash_state = FlashState::Idle;
                    self.flash_data = None;
                }
                if self.flash_state == FlashState::Failed {
                    if ui.button("Clear") {
                        self.flash_state = FlashState::Idle;
                        self.flash_error = None;
                        self.flash_data = None;
                    }
                }

                ui.separator();
                ui.text("F1/XCP (BootCommander)");
                ui.separator();

                ui.text("Interface (can0, peak_pcanusb, /dev/ttyUSB0, etc.):");
                let hint = ctx
                    .connected_interfaces
                    .iter()
                    .find(|(bus, _)| *bus == self.tx_bus)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or("can0");
                let mut iface_str = self.bootcommander_interface.clone();
                if ui.input_text("##bootcommander_iface", &mut iface_str)
                    .hint(hint)
                    .build()
                {
                    self.bootcommander_interface = iface_str.trim().to_string();
                }

                let bc_can_run = !matches!(self.bootcommander_state, BootCommanderState::Running);
                if bc_can_run {
                    if ui.button("Flash via BootCommander (F1)...") {
                        if let Some(path) = FileDialogs::open_firmware_file() {
                            let path = path.to_path_buf();
                            let iface = if self.bootcommander_interface.is_empty() {
                                "can0".to_string()
                            } else {
                                self.bootcommander_interface.clone()
                            };
                            self.bootcommander_state = BootCommanderState::Running;
                            let result_handle = self.bootcommander_result.clone();
                            thread::spawn(move || {
                                let result = run_bootcommander(&path, &iface);
                                if let Ok(mut guard) = result_handle.lock() {
                                    *guard = Some(result);
                                }
                            });
                        }
                    }
                    ui.same_line();
                    ui.text_disabled("(?)");
                    if ui.is_item_hovered() {
                        ui.tooltip(|| {
                            ui.text_wrapped("For F1 (STM32F103) boards. BootCommander is bundled with release builds (cargo build --release). CAN: can0 (Linux) or peak_pcanusb (Windows). Serial (macOS): /dev/cu.usbserial-* or /dev/ttyUSB0 (Linux). Run ./scripts/build-bootcommander.sh if not found.");
                        });
                    }
                } else {
                    ui.text_colored([0.8, 0.8, 0.3, 1.0], "Flashing via BootCommander...");
                }

                match &self.bootcommander_state {
                    BootCommanderState::Done { success, output } => {
                        let color = if *success {
                            [0.3, 0.8, 0.3, 1.0]
                        } else {
                            [1.0, 0.3, 0.3, 1.0]
                        };
                        ui.text_colored(color, if *success { "BootCommander: Done" } else { "BootCommander: Failed" });
                        ui.text_wrapped(&output);
                        if ui.button("Clear##bc") {
                            self.bootcommander_state = BootCommanderState::Idle;
                        }
                    }
                    _ => {}
                }

                ui.separator();
                } // end if ctx.is_connected

                ui.text("Sensor Data");
                ui.separator();

                let mut indices: Vec<u8> = self.sensor_data.keys().copied().collect();
                indices.sort();

                if indices.is_empty() {
                    ui.text_colored([0.6, 0.6, 0.6, 1.0], "No wideband data received yet");
                    ui.text_wrapped("Ensure heater is enabled and wait 10-30s for warm-up. Sensor data uses CAN IDs 0x190-0x1AF.");
                } else {
                    for idx in indices {
                        let data = self.sensor_data.get(&idx).unwrap();
                        let diag = self.diagnostics.get(&idx);

                        let valid_color = if data.valid {
                            [0.3, 0.8, 0.3, 1.0]
                        } else {
                            [0.6, 0.5, 0.3, 1.0]
                        };
                        ui.text_colored(valid_color, format!("Sensor {}", idx));

                        ui.indent();
                        if data.valid {
                            ui.text(format!("  λ: {:.4}", data.lambda_value));
                            ui.text(format!("  AFR: {:.2}", data.afr_gasoline));
                        } else {
                            ui.text_colored(
                                [0.6, 0.6, 0.6, 1.0],
                                "  (warming up or invalid)",
                            );
                        }
                        ui.text(format!("  Temp: {} °C", data.temp_c));

                        if let Some(d) = diag {
                            let status_color = match d.status {
                                STATUS_OK => [0.3, 0.8, 0.3, 1.0],
                                STATUS_HEATER_FAULT => [1.0, 0.3, 0.3, 1.0],
                                STATUS_SENSOR_ERROR => [1.0, 0.5, 0.0, 1.0],
                                _ => [0.6, 0.6, 0.6, 1.0],
                            };
                            ui.text_colored(
                                status_color,
                                format!("  Status: {}", status_name(d.status)),
                            );
                            ui.text(format!(
                                "  ESR: {} Ω  Nernst: {:.3}V",
                                d.esr_ohms, d.nernst_v
                            ));
                            ui.text(format!(
                                "  Pump: {:.0}%  Heater: {:.0}%",
                                d.pump_duty_pct, d.heater_duty_pct
                            ));
                        }
                        ui.unindent();
                    }
                }

                ui.separator();
                ui.text_disabled("Protocol: rusEFI wideband, 500kbps, IDs 0x190+");
            });
    }
}
