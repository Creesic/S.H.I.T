use imgui::{Condition, StyleColor, Ui};
use crate::hardware::can_interface::{CanConfig, CanStatus, InterfaceType};
use chrono::{Utc, Timelike};

/// Live mode state for hardware interface management
pub struct LiveModeState {
    /// Whether live mode is active (connected to hardware)
    pub is_active: bool,
    /// Whether currently recording/capturing data
    pub is_recording: bool,
    /// Selected interface name
    pub selected_interface: Option<String>,
    /// Available interfaces
    pub available_interfaces: Vec<InterfaceInfoUI>,
    /// Connection configuration
    pub config: LiveCanConfig,
    /// Connection status message
    pub status_message: String,
    /// Statistics
    pub stats: LiveStats,
    /// Messages received in live mode
    pub live_messages: Vec<LiveMessage>,
    /// Maximum messages to keep
    pub max_live_messages: usize,
    /// Recording start time
    pub recording_start: Option<chrono::DateTime<Utc>>,
    /// Request to save data
    pub save_requested: bool,
}

/// Interface info for UI
#[derive(Clone)]
pub struct InterfaceInfoUI {
    pub name: String,
    pub interface_type: InterfaceType,
    pub description: String,
    pub available: bool,
}

/// Live CAN configuration
#[derive(Clone, Debug)]
pub struct LiveCanConfig {
    pub bitrate: u32,
    pub listen_only: bool,
    pub auto_start: bool,
}

impl Default for LiveCanConfig {
    fn default() -> Self {
        Self {
            bitrate: 500_000,
            listen_only: true,
            auto_start: true,
        }
    }
}

/// Live statistics
#[derive(Clone, Default)]
pub struct LiveStats {
    pub messages_received: u64,
    pub messages_sent: u64,
    pub errors: u64,
    pub bytes_received: u64,
    pub start_time: Option<chrono::DateTime<Utc>>,
}

/// A single live message
#[derive(Clone)]
pub struct LiveMessage {
    pub timestamp: chrono::DateTime<Utc>,
    pub id: u32,
    pub data: Vec<u8>,
    pub bus: u8,
}

impl LiveModeState {
    pub fn new() -> Self {
        Self {
            is_active: false,
            is_recording: false,
            selected_interface: None,
            available_interfaces: Vec::new(),
            config: LiveCanConfig::default(),
            status_message: String::new(),
            stats: LiveStats::default(),
            live_messages: Vec::new(),
            max_live_messages: 10000,  // Increased for longer recordings
            recording_start: None,
            save_requested: false,
        }
    }

    /// Refresh the list of available interfaces
    pub fn refresh_interfaces(&mut self) {
        // Get serial ports
        let serial_ports = crate::hardware::serial_can::SerialCanInterface::list_serial_ports();

        self.available_interfaces = serial_ports
            .into_iter()
            .map(|name| InterfaceInfoUI {
                name: name.clone(),
                interface_type: InterfaceType::Serial,
                description: format!("Serial: {}", name),
                available: true,
            })
            .collect();

        // Add mock interface for testing
        self.available_interfaces.push(InterfaceInfoUI {
            name: "mock://virtual".to_string(),
            interface_type: InterfaceType::Virtual,
            description: "Virtual/Mock Interface (for testing)".to_string(),
            available: true,
        });

        // Sort by type then name
        self.available_interfaces.sort_by(|a, b| {
            match (a.interface_type, b.interface_type) {
                (InterfaceType::Serial, InterfaceType::Virtual) => std::cmp::Ordering::Less,
                (InterfaceType::Virtual, InterfaceType::Serial) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });
    }

    /// Add a live message
    pub fn add_message(&mut self, id: u32, data: Vec<u8>, bus: u8) {
        let msg = LiveMessage {
            timestamp: Utc::now(),
            id,
            data,
            bus,
        };

        self.live_messages.push(msg);
        self.stats.messages_received += 1;

        // Trim old messages
        while self.live_messages.len() > self.max_live_messages {
            self.live_messages.remove(0);
        }
    }

    /// Clear all live messages
    pub fn clear_messages(&mut self) {
        self.live_messages.clear();
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = LiveStats::default();
        if self.is_active {
            self.stats.start_time = Some(Utc::now());
        }
    }

    /// Get messages per second rate
    pub fn get_rate(&self) -> f64 {
        if let Some(start) = self.stats.start_time {
            let elapsed = (Utc::now() - start).num_milliseconds() as f64 / 1000.0;
            if elapsed > 0.0 {
                return self.stats.messages_received as f64 / elapsed;
            }
        }
        0.0
    }

    /// Start recording
    pub fn start_recording(&mut self) {
        self.is_recording = true;
        self.recording_start = Some(Utc::now());
        self.live_messages.clear();  // Clear previous recording
        self.stats = LiveStats::default();
        self.stats.start_time = Some(Utc::now());
    }

    /// Stop recording
    pub fn stop_recording(&mut self) {
        self.is_recording = false;
        self.recording_start = None;
    }

    /// Get recording duration in seconds
    pub fn recording_duration_secs(&self) -> f64 {
        if let Some(start) = self.recording_start {
            (Utc::now() - start).num_milliseconds() as f64 / 1000.0
        } else {
            0.0
        }
    }

    /// Get recording duration as formatted string
    pub fn recording_duration_formatted(&self) -> String {
        let duration_secs = self.recording_duration_secs();
        let hours = (duration_secs / 3600.0) as u32;
        let minutes = ((duration_secs % 3600.0) / 60.0) as u32;
        let seconds = (duration_secs % 60.0) as u32;
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }

    /// Check if recording is empty
    pub fn has_recorded_data(&self) -> bool {
        !self.live_messages.is_empty()
    }
}

impl Default for LiveModeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Hardware manager window for live mode
pub struct HardwareManagerWindow {
    state: LiveModeState,
    bitrate_input: String,
    show_config: bool,
}

impl HardwareManagerWindow {
    pub fn new() -> Self {
        let mut state = LiveModeState::new();
        state.refresh_interfaces();

        Self {
            bitrate_input: "500000".to_string(),
            state,
            show_config: true,
        }
    }

    pub fn state(&self) -> &LiveModeState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut LiveModeState {
        &mut self.state
    }

    /// Render the hardware manager window
    pub fn render(&mut self, ui: &Ui, is_open: &mut bool) -> LiveModeAction {
        let mut action = LiveModeAction::None;

        ui.window("Hardware Manager")
            .size([350.0, 400.0], Condition::FirstUseEver)
            .position([420.0, 450.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                action = self.render_content(ui);
            });

        action
    }

    /// Render content without window wrapper - for embedding in workspace
    pub fn render_content(&mut self, ui: &Ui) -> LiveModeAction {
        let mut action = LiveModeAction::None;

        // Connection status
        let status_color = if self.state.is_active {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [1.0, 0.5, 0.0, 1.0]
        };

        ui.text_colored(status_color, if self.state.is_active { "● Connected" } else { "○ Disconnected" });
        ui.same_line();
        if ui.small_button("Refresh") {
            self.state.refresh_interfaces();
        }

        ui.separator();

        // Recording controls
        ui.text("Recording:");
        ui.same_line();

        // Recording status indicator
        let recording_color = if self.state.is_recording {
            [1.0, 0.0, 0.0, 1.0]  // Red for recording
        } else {
            [0.5, 0.5, 0.5, 1.0]  // Gray for stopped
        };
        ui.text_colored(recording_color, if self.state.is_recording {
            format!("● REC {}", self.state.recording_duration_formatted())
        } else {
            format!("○ Stopped ({})", self.state.recording_duration_formatted())
        });
        ui.same_line();

        // Start/Stop recording button
        if self.state.is_recording {
            if ui.small_button("Stop") {
                self.state.stop_recording();
                action = LiveModeAction::StopRecording;
            }
        } else {
            let can_record = self.state.is_active;
            let _disabled = if !can_record {
                Some(ui.begin_disabled(true))
            } else {
                None
            };

            if ui.small_button("Start Recording") {
                self.state.start_recording();
                action = LiveModeAction::StartRecording;
            }

            drop(_disabled);
        }
        ui.same_line();

        // Save button
        let can_save = self.state.has_recorded_data();
        let _disabled = if !can_save {
            Some(ui.begin_disabled(true))
        } else {
            None
        };

        if ui.small_button("Save") {
            self.state.save_requested = true;
            action = LiveModeAction::SaveData;
        }

        drop(_disabled);

        ui.separator();

        // Interface selection
        ui.text("Available Interfaces:");
        if self.state.available_interfaces.is_empty() {
            ui.text_colored([0.7, 0.7, 0.7, 1.0], "No interfaces found");
        } else {
            for iface in &self.state.available_interfaces {
                let is_selected = self.state.selected_interface.as_ref() == Some(&iface.name);
                let _tok = if is_selected {
                    Some(ui.push_style_color(StyleColor::Header, [0.3, 0.3, 0.5, 1.0]))
                } else {
                    None
                };

                let type_icon = match iface.interface_type {
                    InterfaceType::Serial => "[USB]",
                    InterfaceType::SocketCan => "[SOC]",
                    InterfaceType::Virtual => "[SIM]",
                    _ => "[???]",
                };

                let label = format!("{} {}", type_icon, iface.name);

                if ui.selectable(&label) {
                    self.state.selected_interface = Some(iface.name.clone());
                }

                drop(_tok);

                // Show description on hover
                if ui.is_item_hovered() {
                    ui.tooltip(|| {
                        ui.text(&iface.description);
                        ui.text(format!("Type: {:?}", iface.interface_type));
                    });
                }
            }
        }

        ui.separator();

        // Configuration
        if ui.collapsing_header("Configuration", imgui::TreeNodeFlags::empty()) {
            // Bitrate
            ui.text("Bitrate:");
            ui.same_line();
            ui.input_text("##bitrate", &mut self.bitrate_input).build();
            if let Ok(val) = self.bitrate_input.parse::<u32>() {
                self.state.config.bitrate = val;
            }

            // Common bitrates
            ui.text("Presets:");
            ui.same_line();
            for &preset in &[125_000, 250_000, 500_000, 1_000_000] {
                if ui.small_button(&format!("{}", preset / 1000)) {
                    self.state.config.bitrate = preset;
                    self.bitrate_input = preset.to_string();
                }
                ui.same_line();
            }
            ui.new_line();

            // Listen only mode
            ui.checkbox("Listen Only Mode", &mut self.state.config.listen_only);
            if ui.is_item_hovered() {
                ui.tooltip(|| {
                    ui.text("When enabled, only receives messages without transmitting");
                });
            }

            // Auto-start
            ui.checkbox("Auto-start Capture", &mut self.state.config.auto_start);
        }

        ui.separator();

        // Connect/Disconnect button
        if self.state.is_active {
            if ui.button("Disconnect") {
                self.state.is_active = false;
                self.state.status_message = "Disconnected".to_string();
                action = LiveModeAction::Disconnect;
            }
        } else {
            let can_connect = self.state.selected_interface.is_some();
            let _disabled = if !can_connect {
                Some(ui.begin_disabled(true))
            } else {
                None
            };

            if ui.button("Connect") {
                if let Some(ref iface) = self.state.selected_interface {
                    self.state.is_active = true;
                    self.state.stats.start_time = Some(Utc::now());
                    self.state.status_message = format!("Connected to {}", iface);
                    action = LiveModeAction::Connect {
                        interface: iface.clone(),
                        config: self.state.config.clone(),
                    };
                }
            }

            drop(_disabled);
        }

        // Status message
        if !self.state.status_message.is_empty() {
            ui.text_colored([0.7, 0.7, 0.7, 1.0], &self.state.status_message);
        }

        ui.separator();

        // Statistics
        if ui.collapsing_header("Statistics", imgui::TreeNodeFlags::empty()) {
            ui.text(format!("Messages Received: {}", self.state.stats.messages_received));
            ui.text(format!("Messages Sent: {}", self.state.stats.messages_sent));
            ui.text(format!("Errors: {}", self.state.stats.errors));
            ui.text(format!("Rate: {:.1} msg/s", self.state.get_rate()));

            if let Some(start) = self.state.stats.start_time {
                let elapsed = (Utc::now() - start).num_seconds();
                ui.text(format!("Running for: {}s", elapsed));
            }

            if ui.small_button("Reset Stats") {
                self.state.reset_stats();
            }
        }

        ui.separator();

        // Live messages preview
        if ui.collapsing_header("Live Messages", imgui::TreeNodeFlags::empty()) {
            // Show last 10 messages
            let count = self.state.live_messages.len().min(10);
            let start = self.state.live_messages.len().saturating_sub(10);

            ui.text(format!("Showing {} of {} messages", count, self.state.live_messages.len()));

            for msg in self.state.live_messages.iter().skip(start) {
                let data_hex: String = msg.data.iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");

                ui.text(format!(
                    "{:02}:{:02}:{:02}.{:03} | 0x{:03X} | {}",
                    msg.timestamp.hour(),
                    msg.timestamp.minute(),
                    msg.timestamp.second(),
                    msg.timestamp.nanosecond() / 1_000_000,
                    msg.id,
                    data_hex
                ));
            }

            if ui.small_button("Clear Messages") {
                self.state.clear_messages();
            }
        }

        action
    }
}

impl Default for HardwareManagerWindow {
    fn default() -> Self {
        Self::new()
    }
}

/// Actions from the hardware manager
#[derive(Clone, Debug)]
pub enum LiveModeAction {
    None,
    Connect {
        interface: String,
        config: LiveCanConfig,
    },
    Disconnect,
    SendMessage {
        id: u32,
        data: Vec<u8>,
    },
    StartRecording,
    StopRecording,
    SaveData,
}

/// Live message list window (separate from manager)
pub struct LiveMessageWindow {
    filter_id: String,
    auto_scroll: bool,
    show_timestamp: bool,
}

impl LiveMessageWindow {
    pub fn new() -> Self {
        Self {
            filter_id: String::new(),
            auto_scroll: true,
            show_timestamp: true,
        }
    }

    pub fn render(&mut self, ui: &Ui, state: &LiveModeState, is_open: &mut bool) {
        ui.window("Live Messages")
            .size([450.0, 350.0], Condition::FirstUseEver)
            .position([780.0, 450.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui, state);
            });
    }

    /// Render content without window wrapper - for embedding in workspace
    pub fn render_content(&mut self, ui: &Ui, state: &LiveModeState) {
        // Filter controls
        ui.text("Filter ID:");
        ui.same_line();
        ui.input_text("##filter", &mut self.filter_id)
            .hint("e.g., 0x123 or 123")
            .build();

        ui.same_line();
        ui.checkbox("Auto-scroll", &mut self.auto_scroll);
        ui.same_line();
        ui.checkbox("Show Timestamp", &mut self.show_timestamp);

        ui.separator();

        // Message count
        ui.text(format!("{} messages", state.live_messages.len()));

        // Use list clipper for performance
        let msg_count = state.live_messages.len() as i32;
        let mut clipper = imgui::ListClipper::new(msg_count).begin(ui);

        while clipper.step() {
            for i in clipper.display_start()..clipper.display_end() {
                let i = i as usize;
                if i >= state.live_messages.len() {
                    continue;
                }

                let msg = &state.live_messages[i];

                // Apply filter
                if !self.filter_id.is_empty() {
                    let filter_lower = self.filter_id.to_lowercase();
                    let id_str = format!("{:03x}", msg.id);
                    if !id_str.contains(&filter_lower) &&
                       !format!("0x{:03x}", msg.id).contains(&filter_lower) {
                        continue;
                    }
                }

                let data_hex: String = msg.data.iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");

                if self.show_timestamp {
                    ui.text(format!(
                        "{:02}:{:02}:{:02}.{:03} | 0x{:03X} | {}",
                        msg.timestamp.hour(),
                        msg.timestamp.minute(),
                        msg.timestamp.second(),
                        msg.timestamp.nanosecond() / 1_000_000,
                        msg.id,
                        data_hex
                    ));
                } else {
                    ui.text(format!("0x{:03X} | {}", msg.id, data_hex));
                }
            }
        }
    }
}

impl Default for LiveMessageWindow {
    fn default() -> Self {
        Self::new()
    }
}

/// Message sender window
pub struct MessageSenderWindow {
    id_input: String,
    data_input: String,
    last_error: Option<String>,
}

impl MessageSenderWindow {
    pub fn new() -> Self {
        Self {
            id_input: "0x000".to_string(),
            data_input: "00 00 00 00 00 00 00 00".to_string(),
            last_error: None,
        }
    }

    pub fn render(&mut self, ui: &Ui, is_connected: bool, is_open: &mut bool) -> Option<(u32, Vec<u8>)> {
        let mut result = None;

        ui.window("Send Message")
            .size([300.0, 180.0], Condition::FirstUseEver)
            .position([780.0, 30.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                result = self.render_content(ui, is_connected);
            });

        result
    }

    /// Render content without window wrapper - for embedding in workspace
    pub fn render_content(&mut self, ui: &Ui, is_connected: bool) -> Option<(u32, Vec<u8>)> {
        if !is_connected {
            ui.text_colored([1.0, 0.5, 0.0, 1.0], "Not connected to CAN interface");
            return None;
        }

        ui.text("CAN ID (hex):");
        ui.same_line();
        ui.input_text("##id", &mut self.id_input)
            .hint("0x123 or 123")
            .build();

        ui.text("Data (hex):");
        ui.same_line();
        ui.input_text("##data", &mut self.data_input)
            .hint("01 02 03 04 05 06 07 08")
            .build();

        if let Some(ref err) = self.last_error {
            ui.text_colored([1.0, 0.3, 0.3, 1.0], err);
        }

        if ui.button("Send") {
            // Parse ID
            let id_str = self.id_input.trim_start_matches("0x").trim_start_matches("0X");
            let id = match u32::from_str_radix(id_str, 16) {
                Ok(v) if v <= 0x7FF || (v <= 0x1FFFFFFF) => v,
                _ => {
                    self.last_error = Some("Invalid CAN ID".to_string());
                    return None;
                }
            };

            // Parse data
            let data: Vec<u8> = self.data_input
                .split_whitespace()
                .filter_map(|s| u8::from_str_radix(s, 16).ok())
                .collect();

            if data.is_empty() || data.len() > 8 {
                self.last_error = Some("Data must be 1-8 bytes".to_string());
                return None;
            }

            self.last_error = None;
            return Some((id, data));
        }

        None
    }
}

impl Default for MessageSenderWindow {
    fn default() -> Self {
        Self::new()
    }
}
