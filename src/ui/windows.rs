use imgui::{Condition, StyleColor, Ui};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::core::CanMessage;
use crate::core::dbc::DbcFile;

/// State tracking for a single CAN message ID
#[derive(Clone, Debug)]
pub struct MessageState {
    pub id: u32,
    pub name: String,
    pub data: Vec<u8>,
    pub byte_colors: Vec<[f32; 4]>,
    pub count: u32,
    pub freq: f32,
    pub last_timestamp: f64,
    pub last_update: Instant,
    // For frequency calculation
    freq_samples: Vec<f64>,
}

impl MessageState {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            name: format!("MSG_0x{:03X}", id),
            data: Vec::new(),
            byte_colors: Vec::new(),
            count: 0,
            freq: 0.0,
            last_timestamp: 0.0,
            last_update: Instant::now(),
            freq_samples: Vec::with_capacity(10),
        }
    }

    pub fn update(&mut self, msg: &CanMessage, msg_name: Option<&str>) {
        // Update name if provided (DBC names override default names)
        if let Some(name) = msg_name {
            if !name.is_empty() {
                self.name = name.to_string();
            }
        }

        // Calculate frequency
        if self.count > 0 && msg.timestamp_unix() > self.last_timestamp {
            let delta = msg.timestamp_unix() - self.last_timestamp;
            if delta > 0.0 {
                self.freq_samples.push(1.0 / delta);
                if self.freq_samples.len() > 10 {
                    self.freq_samples.remove(0);
                }
                // Moving average
                self.freq = self.freq_samples.iter().sum::<f64>() as f32 / self.freq_samples.len() as f32;
            }
        }

        // Update data and calculate colors
        let old_data = self.data.clone();
        self.data = msg.data.clone();
        self.byte_colors = self.calculate_byte_colors(&old_data, &msg.data);

        self.count += 1;
        self.last_timestamp = msg.timestamp_unix();
        self.last_update = Instant::now();
    }

    fn calculate_byte_colors(&self, old_data: &[u8], new_data: &[u8]) -> Vec<[f32; 4]> {
        let mut colors = Vec::with_capacity(new_data.len());

        for (i, &new_byte) in new_data.iter().enumerate() {
            let old_byte = old_data.get(i).copied().unwrap_or(0);
            let diff = new_byte ^ old_byte;

            let color = if self.count == 0 {
                // First message - no change yet
                [0.3, 0.3, 0.35, 1.0]
            } else if diff == 0 {
                // No change
                [0.25, 0.25, 0.28, 1.0]
            } else {
                // Changed - color based on pattern
                let change_ratio = (diff.count_ones() as f32) / 8.0;

                if diff == 0xFF {
                    // All bits changed (toggle?)
                    [0.9, 0.6, 0.2, 1.0] // Orange
                } else if new_byte > old_byte {
                    // Increasing
                    [0.3, 0.7, 0.4, 1.0] // Green
                } else if new_byte < old_byte {
                    // Decreasing
                    [0.7, 0.4, 0.3, 1.0] // Red
                } else {
                    // Mixed change
                    [0.5, 0.5, 0.2 + change_ratio * 0.5, 1.0] // Yellow-ish
                }
            };
            colors.push(color);
        }

        colors
    }

    pub fn hex_data(&self) -> String {
        self.data.iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn freq_str(&self) -> String {
        if self.freq >= 1000.0 {
            format!("{:.1} kHz", self.freq / 1000.0)
        } else if self.freq >= 1.0 {
            format!("{:.1} Hz", self.freq)
        } else {
            format!("{:.0} mHz", self.freq * 1000.0)
        }
    }

    pub fn is_active(&self) -> bool {
        self.last_update.elapsed() < Duration::from_millis(500)
    }
}

/// Window showing live CAN message state - one row per CAN ID (Cabana style)
pub struct MessageListWindow {
    /// Map of CAN ID to current state
    states: HashMap<u32, MessageState>,
    /// All messages (for full history mode)
    messages: Vec<CanMessage>,
    /// Selected CAN ID
    selected_id: Option<u32>,
    /// Display mode
    live_mode: bool,
    /// Filter string
    filter: String,
    /// Sort column
    sort_column: usize,
    sort_ascending: bool,
    /// DBC file for message names
    dbc_file: Option<DbcFile>,
}

impl MessageListWindow {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            messages: Vec::new(),
            selected_id: None,
            live_mode: true,
            filter: String::new(),
            sort_column: 0,
            sort_ascending: true,
            dbc_file: None,
        }
    }

    pub fn set_messages(&mut self, messages: Vec<CanMessage>) {
        self.messages = messages;
    }

    pub fn set_dbc(&mut self, dbc: DbcFile) {
        self.dbc_file = Some(dbc);

        // Update all existing message names with DBC names
        if let Some(ref dbc) = self.dbc_file {
            for (&msg_id, state) in self.states.iter_mut() {
                if let Some(msg_def) = dbc.get_message(msg_id) {
                    // Update to DBC message name
                    state.name = msg_def.name.clone();
                }
            }
        }
    }

    /// Update state with a new message (called during playback)
    pub fn update_message(&mut self, msg: &CanMessage) {
        let state = self.states.entry(msg.id).or_insert_with(|| MessageState::new(msg.id));

        // Get message name from DBC if available
        let msg_name = self.dbc_file.as_ref()
            .and_then(|dbc| dbc.get_message(msg.id))
            .map(|m| m.name.as_str());

        state.update(msg, msg_name);
    }

    /// Clear all states
    pub fn clear(&mut self) {
        self.states.clear();
        self.messages.clear();
        self.selected_id = None;
    }

    pub fn selected_message(&self) -> Option<&MessageState> {
        self.selected_id.and_then(|id| self.states.get(&id))
    }

    /// Debug info
    pub fn debug_info(&self) -> (Option<u32>, usize, usize) {
        (self.selected_id, self.states.len(), self.messages.len())
    }

    pub fn render(&mut self, ui: &Ui, is_open: &mut bool) {
        ui.window("Messages")
            .size([500.0, 400.0], Condition::FirstUseEver)
            .position([10.0, 30.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui);
            });
    }

    pub fn render_content(&mut self, ui: &Ui) {
        // Mode toggle
        let mut mode = if self.live_mode { 0 } else { 1 };
        if ui.radio_button("Live", &mut mode, 0) {
            self.live_mode = mode == 0;
        }
        ui.same_line();
        if ui.radio_button("History", &mut mode, 1) {
            self.live_mode = mode == 0;
        }

        ui.same_line();
        ui.spacing();
        ui.same_line();

        if ui.small_button("Clear") {
            self.clear();
        }

        ui.same_line();

        // Filter
        ui.text("Filter:");
        ui.same_line();
        let _ = ui.input_text("##filter", &mut self.filter)
            .hint("ID or name...")
            .build();

        ui.separator();

        if self.live_mode {
            self.render_live_mode(ui);
        } else {
            self.render_history_mode(ui);
        }
    }

    fn render_live_mode(&mut self, ui: &Ui) {
        // Header - use auto-sizing columns with manual width constraints
        ui.columns(5, "msg_header", false);

        // ID column - fixed width for hex ID
        ui.set_column_width(0, 60.0);
        ui.text("ID"); ui.next_column();

        // Name column - gets remaining space
        ui.text("Name"); ui.next_column();

        // Freq column - fixed width for frequency
        ui.set_column_width(2, 50.0);
        ui.text("Freq"); ui.next_column();

        // Count column - fixed width for count
        ui.set_column_width(3, 50.0);
        ui.text("Count"); ui.next_column();

        // Data column - gets remaining space
        ui.text("Data"); ui.next_column();
        ui.separator();

        // Collect and sort states
        let filter_lower = self.filter.to_lowercase();
        let mut sorted_ids: Vec<u32> = self.states.keys().cloned().collect();

        // Apply filter
        if !filter_lower.is_empty() {
            sorted_ids.retain(|id| {
                if let Some(state) = self.states.get(id) {
                    let id_str = format!("0x{:03X}", id);
                    let name_lower = state.name.to_lowercase();
                    id_str.to_lowercase().contains(&filter_lower) || name_lower.contains(&filter_lower)
                } else {
                    false
                }
            });
        }

        // Sort
        sorted_ids.sort_by(|a, b| {
            let state_a = self.states.get(a).unwrap();
            let state_b = self.states.get(b).unwrap();
            let cmp = match self.sort_column {
                0 => a.cmp(b),
                1 => state_a.name.cmp(&state_b.name),
                2 => state_a.freq.partial_cmp(&state_b.freq).unwrap_or(std::cmp::Ordering::Equal),
                3 => state_a.count.cmp(&state_b.count),
                _ => a.cmp(b),
            };
            if self.sort_ascending { cmp } else { cmp.reverse() }
        });

        // Render rows
        for id in sorted_ids {
            let state = self.states.get(&id).unwrap();
            let is_selected = self.selected_id == Some(id);
            let is_active = state.is_active();

            // Highlight color for active/selected
            let _token = if is_selected {
                Some(ui.push_style_color(StyleColor::Header, [0.3, 0.3, 0.5, 1.0]))
            } else if is_active {
                Some(ui.push_style_color(StyleColor::Header, [0.25, 0.35, 0.25, 1.0]))
            } else {
                None
            };

            // ID column - selectable spanning all columns
            let id_str = format!("0x{:03X}", id);
            if ui.selectable_config(&id_str).span_all_columns(true).build() {
                self.selected_id = Some(id);
            }
            ui.next_column();

            // Name column
            if is_active {
                ui.text_colored([0.5, 1.0, 0.5, 1.0], &state.name);
            } else {
                ui.text(&state.name);
            }
            ui.next_column();

            // Freq column
            ui.text(&state.freq_str());
            ui.next_column();

            // Count column
            ui.text(format!("{}", state.count));
            ui.next_column();

            // Data column - colored hex bytes
            self.render_colored_bytes(ui, state);
            ui.next_column();

            drop(_token);
        }

        ui.columns(1, "", false);

        // Show selected message details
        if let Some(state) = self.selected_message() {
            ui.separator();
            self.render_message_details(ui, state);
        }
    }

    fn render_colored_bytes(&self, ui: &Ui, state: &MessageState) {
        let draw_list = ui.get_window_draw_list();
        let cursor = ui.cursor_screen_pos();

        let byte_width = 22.0;
        let byte_height = 18.0;
        let gap = 2.0;

        for (i, (&byte, &color)) in state.data.iter().zip(state.byte_colors.iter()).enumerate() {
            // Add gap every 4 bytes
            let gap_offset = (i / 4) as f32 * 4.0;

            let x = cursor[0] + (i as f32 * byte_width) + gap_offset;
            let y = cursor[1];

            // Background color
            draw_list.add_rect(
                [x, y],
                [x + byte_width - gap, y + byte_height],
                color,
            ).filled(true).rounding(2.0).build();

            // Hex text
            let hex = format!("{:02X}", byte);
            let text_color = if color[0] + color[1] + color[2] > 1.5 {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            draw_list.add_text([x + 3.0, y + 2.0], text_color, hex);
        }

        // Reserve space
        let total_width = (state.data.len() as f32 * byte_width) + ((state.data.len() / 4) as f32 * 4.0);
        ui.dummy([total_width.max(100.0), byte_height]);
    }

    fn render_message_details(&self, ui: &Ui, state: &MessageState) {
        ui.text(format!("Message: {} (0x{:03X})", state.name, state.id));
        ui.text(format!("Frequency: {}", state.freq_str()));
        ui.text(format!("Count: {}", state.count));

        ui.separator();
        ui.text("Data bytes:");

        // Show detailed byte view
        ui.indent();
        for (i, (&byte, &color)) in state.data.iter().zip(state.byte_colors.iter()).enumerate() {
            ui.text_colored(color, format!("[{:2}] {:02X} ({:3})", i, byte, byte));
        }
        ui.unindent();
    }

    fn render_history_mode(&mut self, ui: &Ui) {
        ui.text_wrapped("History mode shows all recorded messages.");
        ui.text(format!("Total messages: {}", self.messages.len()));

        let mut clipper = imgui::ListClipper::new(self.messages.len() as i32).begin(ui);

        while clipper.step() {
            for i in clipper.display_start()..clipper.display_end() {
                let i = i as usize;
                if let Some(msg) = self.messages.get(i) {
                    let label = format!(
                        "{} | 0x{:03X} | {}",
                        msg.timestamp.format("%H:%M:%S%.3f"),
                        msg.id,
                        msg.hex_data()
                    );

                    if ui.selectable(&label) {
                        self.selected_id = Some(msg.id);
                    }
                }
            }
        }
    }
}

/// Window for editing DBC file definitions
pub struct DbcEditorWindow {
    dbc_file: DbcFile,
    selected_message: Option<u32>,
    /// Pending load request
    pub load_requested: bool,
    /// Pending save request
    pub save_requested: bool,
}

impl DbcEditorWindow {
    pub fn new() -> Self {
        Self {
            dbc_file: DbcFile::new(),
            selected_message: None,
            load_requested: false,
            save_requested: false,
        }
    }

    pub fn set_dbc(&mut self, dbc_file: DbcFile) {
        self.dbc_file = dbc_file;
    }

    pub fn get_dbc(&self) -> &DbcFile {
        &self.dbc_file
    }

    pub fn render(&mut self, ui: &Ui, is_open: &mut bool) {
        ui.window("DBC Editor")
            .size([400.0, 400.0], Condition::FirstUseEver)
            .position([10.0, 450.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui);
            });
    }

    pub fn render_content(&mut self, ui: &Ui) {
        ui.text("DBC File Editor");
        ui.text("Load a .dbc file to edit signal definitions");

        ui.separator();

        if ui.button("Load DBC") {
            self.load_requested = true;
        }

        ui.same_line();
        if ui.button("Save DBC") {
            self.save_requested = true;
        }

        ui.separator();

        ui.text(format!("Messages: {} defined", self.dbc_file.messages.len()));

        for msg in &self.dbc_file.messages {
            let is_selected = self.selected_message == Some(msg.id);
            let label = format!("0x{:03X} - {} ({})", msg.id, msg.name, msg.size);

            let _token = if is_selected {
                Some(ui.push_style_color(StyleColor::Header, [0.3, 0.3, 0.4, 1.0]))
            } else {
                None
            };

            if ui.selectable(&label) {
                self.selected_message = Some(msg.id);
            }

            drop(_token);
        }

        ui.separator();

        // Show selected message details
        if let Some(msg_id) = self.selected_message {
            if let Some(msg) = self.dbc_file.get_message(msg_id) {
                ui.text(format!("Message: {}", msg.name));
                ui.text(format!("  ID: 0x{:03X}", msg.id));
                ui.text(format!("  Size: {} bytes", msg.size));
                ui.text(format!("  Signals: {}", msg.signals.len()));

                ui.separator();

                ui.text("Signals:");
                for signal in &msg.signals {
                    ui.text(format!("  - {}", signal.name));
                    ui.text(format!(
                        "    Start bit: {}, Length: {}",
                        signal.start_bit, signal.bit_length
                    ));
                    ui.text(format!(
                        "    Factor: {}, Offset: {}",
                        signal.factor, signal.offset
                    ));
                    if let Some(ref unit) = signal.unit {
                        ui.text(format!("    Unit: {}", unit));
                    }
                }
            }
        }
    }
}
