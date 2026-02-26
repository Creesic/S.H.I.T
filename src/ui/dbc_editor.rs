use imgui::{Condition, StyleColor, Ui, TreeNodeFlags};
use crate::core::dbc::{DbcFile, DbcMessage, DbcSignal, ByteOrder, ValueType};

/// Enhanced DBC editor for reverse engineering
pub struct DbcEditorEnhanced {
    dbc_file: DbcFile,
    selected_message_id: Option<u32>,
    selected_signal_name: Option<String>,
    show_bit_editor: bool,
    show_validation: bool,
    validation_errors: Vec<String>,
    // New message/signal fields
    new_message_id: String,
    new_message_name: String,
    new_message_size: String,
    new_signal_name: String,
    new_signal_start_bit: String,
    new_signal_bit_length: String,
    new_signal_factor: String,
    new_signal_offset: String,
    // Editing state for radio buttons
    edit_byte_order_intel: bool,
    edit_value_type_unsigned: bool,
}

impl DbcEditorEnhanced {
    pub fn new() -> Self {
        Self {
            dbc_file: DbcFile::new(),
            selected_message_id: None,
            selected_signal_name: None,
            show_bit_editor: true,
            show_validation: false,
            validation_errors: Vec::new(),
            new_message_id: String::new(),
            new_message_name: String::new(),
            new_message_size: String::from("8"),
            new_signal_name: String::new(),
            new_signal_start_bit: String::from("0"),
            new_signal_bit_length: String::from("8"),
            new_signal_factor: String::from("1"),
            new_signal_offset: String::from("0"),
            edit_byte_order_intel: true,
            edit_value_type_unsigned: true,
        }
    }

    pub fn set_dbc(&mut self, dbc_file: DbcFile) {
        self.dbc_file = dbc_file;
        self.selected_message_id = None;
        self.selected_signal_name = None;
        self.validation_errors.clear();
    }

    pub fn get_dbc(&self) -> &DbcFile {
        &self.dbc_file
    }

    /// Validate the current DBC file
    pub fn validate(&mut self) {
        self.validation_errors.clear();

        for msg in &self.dbc_file.messages {
            // Check message size
            if msg.size == 0 {
                self.validation_errors.push(format!(
                    "Message {} (0x{:03X}): DLC is 0",
                    msg.name, msg.id
                ));
            }
            if msg.size > 8 {
                self.validation_errors.push(format!(
                    "Message {} (0x{:03X}): DLC {} > 8 (not CAN 2.0 compliant)",
                    msg.name, msg.id, msg.size
                ));
            }

            // Check for overlapping signals
            for i in 0..msg.signals.len() {
                for j in (i + 1)..msg.signals.len() {
                    if signals_overlap(&msg.signals[i], &msg.signals[j]) {
                        self.validation_errors.push(format!(
                            "Message {}: Signals '{}' and '{}' overlap",
                            msg.name, msg.signals[i].name, msg.signals[j].name
                        ));
                    }
                }
            }

            // Check signal bounds
            for signal in &msg.signals {
                let end_bit = signal.start_bit + signal.bit_length - 1;

                if end_bit >= (msg.size as u8 * 8) {
                    self.validation_errors.push(format!(
                        "Message {} / Signal {}: Extends beyond message DLC",
                        msg.name, signal.name
                    ));
                }
            }
        }
    }

    /// Render the DBC editor
    pub fn render(&mut self, ui: &Ui) {
        ui.window("DBC Editor (Enhanced)")
            .size([900.0, 600.0], Condition::FirstUseEver)
            .build(|| {
                // Left panel: Message list
                ui.columns(3, "dbc_columns", true);
                self.render_message_list(ui);

                // Middle panel: Signal list
                ui.next_column();
                self.render_signal_list(ui);

                // Right panel: Signal editor
                ui.next_column();
                self.render_signal_editor(ui);

                ui.columns(1, "", false);

                // Bit editor at bottom
                if self.show_bit_editor && self.selected_message_id.is_some() {
                    ui.separator();
                    self.render_bit_editor(ui);
                }

                // Validation panel
                if self.show_validation {
                    ui.separator();
                    self.render_validation_panel(ui);
                }
            });
    }

    fn render_message_list(&mut self, ui: &Ui) {
        ui.text("Messages");
        ui.separator();

        // Add new message
        if ui.collapsing_header("Add Message", TreeNodeFlags::empty()) {
            ui.input_text("ID (hex)", &mut self.new_message_id)
                .hint("0x123")
                .build();
            ui.input_text("Name", &mut self.new_message_name)
                .hint("MessageName")
                .build();
            ui.input_text("Size (bytes)", &mut self.new_message_size)
                .build();

            if ui.button("Add") {
                if let Ok(id) = u32::from_str_radix(self.new_message_id.trim_start_matches("0x"), 16) {
                    if let Ok(size) = self.new_message_size.parse::<u8>() {
                        let msg = DbcMessage::new(id, &self.new_message_name, size);
                        self.dbc_file.add_message(msg);
                        self.new_message_id.clear();
                        self.new_message_name.clear();
                    }
                }
            }
            ui.separator();
        }

        // Message list (collect data first to avoid borrow issues)
        let msg_data: Vec<(u32, String, usize)> = self.dbc_file.messages.iter()
            .map(|m| (m.id, m.name.clone(), m.signals.len()))
            .collect();

        for (msg_id, msg_name, signal_count) in msg_data {
            let is_selected = self.selected_message_id == Some(msg_id);
            let _tok = if is_selected {
                Some(ui.push_style_color(StyleColor::Header, [0.3, 0.3, 0.5, 1.0]))
            } else {
                None
            };

            let label = format!("0x{:03X} - {}", msg_id, msg_name);
            if ui.selectable(&label) {
                self.selected_message_id = Some(msg_id);
                self.selected_signal_name = None;
            }

            drop(_tok);

            // Context menu
            if let Some(_popup) = ui.begin_popup_context_item() {
                if ui.selectable("Delete") {
                    self.dbc_file.remove_message(msg_id);
                    if self.selected_message_id == Some(msg_id) {
                        self.selected_message_id = None;
                    }
                }
                if ui.selectable("Duplicate") {
                    if let Some(msg) = self.dbc_file.get_message(msg_id) {
                        let mut new_msg = msg.clone();
                        new_msg.id = msg_id + 1;
                        new_msg.name = format!("{}_copy", msg_name);
                        self.dbc_file.add_message(new_msg);
                    }
                }
            }

            // Show signal count
            ui.same_line();
            ui.text_colored([0.6, 0.6, 0.6, 1.0], format!("({})", signal_count));
        }

        ui.text(format!("\n{} messages defined", self.dbc_file.messages.len()));
    }

    fn render_signal_list(&mut self, ui: &Ui) {
        ui.text("Signals");
        ui.separator();

        let selected_id = match self.selected_message_id {
            Some(id) => id,
            None => {
                ui.text("Select a message to view signals");
                return;
            }
        };

        // Add new signal section
        if ui.collapsing_header("Add Signal", TreeNodeFlags::empty()) {
            ui.input_text("Name", &mut self.new_signal_name)
                .hint("SignalName")
                .build();
            ui.input_text("Start bit", &mut self.new_signal_start_bit).build();
            ui.input_text("Bit length", &mut self.new_signal_bit_length).build();
            ui.input_text("Factor", &mut self.new_signal_factor).build();
            ui.input_text("Offset", &mut self.new_signal_offset).build();

            if ui.button("Add Signal") {
                if let (Ok(start), Ok(len), Ok(factor), Ok(offset)) = (
                    self.new_signal_start_bit.parse::<u8>(),
                    self.new_signal_bit_length.parse::<u8>(),
                    self.new_signal_factor.parse::<f64>(),
                    self.new_signal_offset.parse::<f64>(),
                ) {
                    let signal = DbcSignal::with_options(
                        &self.new_signal_name,
                        start,
                        len,
                        ByteOrder::Intel,
                        ValueType::Unsigned,
                        factor,
                        offset,
                    );
                    if let Some(msg) = self.dbc_file.get_message_mut(selected_id) {
                        msg.add_signal(signal);
                    }
                    self.new_signal_name.clear();
                }
            }
            ui.separator();
        }

        // Collect signal data first to avoid borrow issues
        let signal_data: Vec<(String, u8, u8, bool)> = match self.dbc_file.get_message(selected_id) {
            Some(msg) => msg.signals.iter()
                .map(|s| (s.name.clone(), s.start_bit, s.bit_length, s.byte_order == ByteOrder::Intel))
                .collect(),
            None => return,
        };

        // Signal list
        for (signal_name, start_bit, bit_length, is_intel) in signal_data {
            let is_selected = self.selected_signal_name == Some(signal_name.clone());
            let _tok = if is_selected {
                Some(ui.push_style_color(StyleColor::Header, [0.3, 0.5, 0.3, 1.0]))
            } else {
                None
            };

            let label = format!("{} [{}:{}] ({})", signal_name, start_bit,
                start_bit + bit_length - 1, if is_intel { "i" } else { "m" });

            if ui.selectable(&label) {
                self.selected_signal_name = Some(signal_name.clone());
                self.edit_byte_order_intel = is_intel;
                // Get value type from signal
                if let Some(msg) = self.dbc_file.get_message(selected_id) {
                    if let Some(sig) = msg.signals.iter().find(|s| s.name == signal_name) {
                        self.edit_value_type_unsigned = sig.value_type == ValueType::Unsigned;
                    }
                }
            }

            drop(_tok);

            // Context menu for deletion
            if let Some(_popup) = ui.begin_popup_context_item() {
                if ui.selectable("Delete") {
                    if let Some(msg) = self.dbc_file.get_message_mut(selected_id) {
                        msg.signals.retain(|s| s.name != signal_name);
                    }
                    if self.selected_signal_name == Some(signal_name.clone()) {
                        self.selected_signal_name = None;
                    }
                }
            }
        }
    }

    fn render_signal_editor(&mut self, ui: &Ui) {
        ui.text("Signal Details");
        ui.separator();

        let (selected_id, signal_name) = match (self.selected_message_id, &self.selected_signal_name) {
            (Some(id), Some(name)) => (id, name.clone()),
            _ => {
                ui.text("Select a signal to edit");
                return;
            }
        };

        // Get mutable access to the signal
        let signal = match self.dbc_file.get_message_mut(selected_id) {
            Some(msg) => msg.signals.iter_mut().find(|s| s.name == signal_name),
            None => return,
        };

        let signal = match signal {
            Some(s) => s,
            None => return,
        };

        // Name
        let mut name = signal.name.clone();
        ui.text("Name:");
        ui.same_line();
        ui.input_text("##name", &mut name).build();
        if name != signal.name && !name.is_empty() {
            signal.name = name;
        }

        // Start bit
        let mut start_str = signal.start_bit.to_string();
        ui.text("Start bit:");
        ui.same_line();
        ui.input_text("##start", &mut start_str).build();
        if let Ok(val) = start_str.parse::<u8>() {
            signal.start_bit = val;
        }

        // Bit length
        let mut len_str = signal.bit_length.to_string();
        ui.text("Length:");
        ui.same_line();
        ui.input_text("##len", &mut len_str).build();
        if let Ok(val) = len_str.parse::<u8>() {
            signal.bit_length = val;
        }

        // Factor
        let mut factor_str = signal.factor.to_string();
        ui.text("Factor:");
        ui.same_line();
        ui.input_text("##factor", &mut factor_str).build();
        if let Ok(val) = factor_str.parse::<f64>() {
            signal.factor = val;
        }

        // Offset
        let mut offset_str = signal.offset.to_string();
        ui.text("Offset:");
        ui.same_line();
        ui.input_text("##offset", &mut offset_str).build();
        if let Ok(val) = offset_str.parse::<f64>() {
            signal.offset = val;
        }

        // Byte order
        ui.text("Byte Order:");
        if ui.radio_button("Intel (Little-endian)", &mut self.edit_byte_order_intel, true) {
            signal.byte_order = ByteOrder::Intel;
        }
        if ui.radio_button("Motorola (Big-endian)", &mut self.edit_byte_order_intel, false) {
            signal.byte_order = ByteOrder::Motorola;
        }

        // Value type
        ui.text("Value Type:");
        if ui.radio_button("Unsigned", &mut self.edit_value_type_unsigned, true) {
            signal.value_type = ValueType::Unsigned;
        }
        if ui.radio_button("Signed", &mut self.edit_value_type_unsigned, false) {
            signal.value_type = ValueType::Signed;
        }

        // Unit
        let mut unit = signal.unit.clone().unwrap_or_default();
        ui.text("Unit:");
        ui.same_line();
        ui.input_text("##unit", &mut unit).build();
        signal.unit = if unit.is_empty() { None } else { Some(unit) };

        // Range
        ui.text(format!("Raw range: {} to {}", signal.raw_range().0, signal.raw_range().1));
        ui.text(format!("Physical range: {:.2} to {:.2}", signal.physical_range().0, signal.physical_range().1));
    }

    fn render_bit_editor(&mut self, ui: &Ui) {
        let selected_id = match self.selected_message_id {
            Some(id) => id,
            None => return,
        };

        let msg = match self.dbc_file.get_message(selected_id) {
            Some(m) => m,
            None => return,
        };

        ui.text(format!("Bit Layout - {} (0x{:03X}) - {} bytes", msg.name, msg.id, msg.size));
        ui.separator();

        let draw_list = ui.get_window_draw_list();
        let cursor = ui.cursor_screen_pos();
        let cell_size = 20.0;
        let gap = 2.0;
        let label_width = 30.0;

        // Draw byte labels on left
        for byte_idx in 0..msg.size as usize {
            let y = cursor[1] + byte_idx as f32 * (cell_size + gap);
            draw_list.add_text(
                [cursor[0], y + 4.0],
                [0.7, 0.7, 0.7, 1.0],
                format!("B{}", byte_idx),
            );
        }

        // Draw bit grid
        for byte_idx in 0..msg.size as usize {
            for bit_idx in 0..8 {
                let bit_pos = byte_idx * 8 + bit_idx;
                let x = cursor[0] + label_width + bit_idx as f32 * (cell_size + gap);
                let y = cursor[1] + byte_idx as f32 * (cell_size + gap);

                // Find which signal owns this bit
                let (owner_color, _owner_name) = self.get_bit_info(&msg, bit_pos as u8);

                // Draw cell
                draw_list.add_rect(
                    [x, y],
                    [x + cell_size, y + cell_size],
                    owner_color,
                ).filled(true).rounding(2.0).build();

                // Draw bit number
                draw_list.add_text(
                    [x + 5.0, y + 4.0],
                    [0.0, 0.0, 0.0, 1.0],
                    format!("{}", bit_idx),
                );
            }
        }

        // Reserve space
        let total_height = msg.size as f32 * (cell_size + gap);
        ui.dummy([label_width + 8.0 * (cell_size + gap), total_height]);

        // Legend
        ui.separator();
        ui.text("Signals:");
        for signal in &msg.signals {
            let color = self.get_signal_color(&signal.name);
            ui.color_button(&signal.name, color);
            ui.same_line();
            ui.text(format!("{} [{}:{}]", signal.name, signal.start_bit,
                signal.start_bit + signal.bit_length - 1));
        }
    }

    fn get_bit_info(&self, msg: &DbcMessage, bit_pos: u8) -> ([f32; 4], Option<String>) {
        for signal in &msg.signals {
            let start = signal.start_bit;
            let end = start + signal.bit_length - 1;
            if bit_pos >= start && bit_pos <= end {
                return (self.get_signal_color(&signal.name), Some(signal.name.clone()));
            }
        }
        ([0.3, 0.3, 0.3, 1.0], None)
    }

    fn get_signal_color(&self, name: &str) -> [f32; 4] {
        // Generate a consistent color based on signal name hash
        let hash = name.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
        let colors = [
            [0.2, 0.6, 0.8, 0.8],
            [0.8, 0.4, 0.2, 0.8],
            [0.3, 0.7, 0.3, 0.8],
            [0.7, 0.3, 0.7, 0.8],
            [0.7, 0.7, 0.2, 0.8],
            [0.4, 0.2, 0.7, 0.8],
            [0.2, 0.7, 0.7, 0.8],
            [0.7, 0.2, 0.4, 0.8],
        ];
        colors[(hash as usize) % colors.len()]
    }

    fn render_validation_panel(&mut self, ui: &Ui) {
        ui.text("Validation Results");
        ui.same_line();
        if ui.small_button("Re-validate") {
            self.validate();
        }
        ui.separator();

        if self.validation_errors.is_empty() {
            ui.text_colored([0.0, 1.0, 0.0, 1.0], "No validation errors");
        } else {
            ui.text_colored([1.0, 0.3, 0.3, 1.0],
                format!("{} error(s) found", self.validation_errors.len()));

            for error in &self.validation_errors {
                ui.bullet_text(error);
            }
        }
    }
}

/// Check if two signals overlap
fn signals_overlap(a: &DbcSignal, b: &DbcSignal) -> bool {
    let a_start = a.start_bit;
    let a_end = a.start_bit + a.bit_length - 1;
    let b_start = b.start_bit;
    let b_end = b.start_bit + b.bit_length - 1;

    // Simple overlap check (assumes same byte order)
    a_start <= b_end && b_start <= a_end
}

impl Default for DbcEditorEnhanced {
    fn default() -> Self {
        Self::new()
    }
}

/// Value table editor for signal enums
pub struct ValueTableEditor {
    signal_name: String,
    values: Vec<(u32, String)>,
    new_value: String,
    new_description: String,
}

impl ValueTableEditor {
    pub fn new() -> Self {
        Self {
            signal_name: String::new(),
            values: Vec::new(),
            new_value: String::new(),
            new_description: String::new(),
        }
    }

    pub fn set_signal(&mut self, name: &str, values: &[(u32, String)]) {
        self.signal_name = name.to_string();
        self.values = values.to_vec();
    }

    pub fn render(&mut self, ui: &Ui) {
        ui.window("Value Table Editor")
            .size([400.0, 300.0], Condition::FirstUseEver)
            .build(|| {
                ui.text(format!("Signal: {}", self.signal_name));
                ui.separator();

                // Add new value
                ui.input_text("Value", &mut self.new_value).build();
                ui.input_text("Description", &mut self.new_description).build();
                if ui.button("Add") {
                    if let Ok(val) = self.new_value.parse::<u32>() {
                        self.values.push((val, self.new_description.clone()));
                        self.new_value.clear();
                        self.new_description.clear();
                    }
                }

                ui.separator();

                // Value list
                let mut to_remove = None;
                for (i, (val, desc)) in self.values.iter().enumerate() {
                    ui.text(format!("{} = {}", val, desc));
                    ui.same_line();
                    if ui.small_button(&format!("X##{}", i)) {
                        to_remove = Some(i);
                    }
                }
                if let Some(idx) = to_remove {
                    self.values.remove(idx);
                }
            });
    }
}

impl Default for ValueTableEditor {
    fn default() -> Self {
        Self::new()
    }
}
