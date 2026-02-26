use imgui::{Condition, StyleColor, Ui};
use crate::core::dbc::{DbcFile, DbcMessage, DbcSignal, ByteOrder, ValueType};
use crate::decode::decoder::extract_bits;
use std::cell::RefCell;

/// Signal color palette for visualizing different signals (more vibrant)
const SIGNAL_COLORS: [[f32; 4]; 10] = [
    [0.3, 0.5, 0.9, 0.7],  // Blue
    [0.3, 0.7, 0.4, 0.7],  // Green
    [0.9, 0.6, 0.2, 0.7],  // Orange
    [0.7, 0.4, 0.8, 0.7],  // Purple
    [0.8, 0.3, 0.4, 0.7],  // Red
    [0.3, 0.8, 0.8, 0.7],  // Cyan
    [0.8, 0.8, 0.3, 0.7],  // Yellow
    [0.6, 0.4, 0.3, 0.7],  // Brown
    [0.5, 0.5, 0.7, 0.7],  // Slate
    [0.7, 0.5, 0.7, 0.7],  // Mauve
];

/// Callback type for when a signal is created
pub type SignalCreatedCallback = Box<dyn FnMut(u32, DbcSignal)>;

/// Callback type for toggling a signal on the chart
pub type ToggleChartCallback = Box<dyn FnMut(&str)>;

/// Window for visualizing CAN message bytes and bits in a grid format
pub struct BitVisualizerWindow {
    /// Currently displayed message ID
    selected_message_id: Option<u32>,
    /// Currently displayed bus ID
    selected_bus: Option<u8>,
    /// Current message data (padded to 8 bytes)
    current_data: [u8; 8],
    /// Show signal overlays
    show_signals: bool,

    // Selection state
    selection_start: Option<usize>,
    selection_end: Option<usize>,
    is_dragging: bool,

    // Signal creation dialog
    show_create_dialog: bool,
    new_signal_name: String,
    new_signal_is_signed: bool,
    new_signal_is_little_endian: bool,
    new_signal_factor: String,
    new_signal_offset: String,
    new_signal_unit: String,
    signal_counter: u32,

    // Signal editing
    show_edit_dialog: bool,
    editing_signal_name: String,
    editing_signal_idx: Option<usize>,
    edit_start_bit: u8,
    edit_bit_length: u8,
    edit_is_signed: bool,
    edit_is_little_endian: bool,
    edit_factor: String,
    edit_offset: String,
    edit_unit: String,

    // Activity tracking (heatmap)
    bit_flip_counts: [u32; 64],
    last_data: [u8; 8],
    max_flip_count: u32,

    // Callbacks
    on_signal_created: RefCell<Option<SignalCreatedCallback>>,
    on_toggle_chart: RefCell<Option<ToggleChartCallback>>,
    charted_signals: RefCell<Vec<String>>,
    chart_toggle_request: RefCell<Option<String>>,
}

impl BitVisualizerWindow {
    pub fn new() -> Self {
        Self {
            selected_message_id: None,
            selected_bus: None,
            current_data: [0; 8],
            show_signals: true,
            selection_start: None,
            selection_end: None,
            is_dragging: false,
            show_create_dialog: false,
            new_signal_name: String::new(),
            new_signal_is_signed: false,
            new_signal_is_little_endian: true,
            new_signal_factor: String::from("1"),
            new_signal_offset: String::from("0"),
            new_signal_unit: String::new(),
            signal_counter: 0,
            show_edit_dialog: false,
            editing_signal_name: String::new(),
            editing_signal_idx: None,
            edit_start_bit: 0,
            edit_bit_length: 1,
            edit_is_signed: false,
            edit_is_little_endian: true,
            edit_factor: String::from("1"),
            edit_offset: String::from("0"),
            edit_unit: String::new(),
            bit_flip_counts: [0; 64],
            last_data: [0; 8],
            max_flip_count: 0,
            on_signal_created: RefCell::new(None),
            on_toggle_chart: RefCell::new(None),
            charted_signals: RefCell::new(Vec::new()),
            chart_toggle_request: RefCell::new(None),
        }
    }

    pub fn set_on_signal_created<F>(&self, callback: F)
    where
        F: FnMut(u32, DbcSignal) + 'static,
    {
        *self.on_signal_created.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for toggling signals on charts
    pub fn set_on_toggle_chart<F>(&self, callback: F)
    where
        F: FnMut(&str) + 'static,
    {
        *self.on_toggle_chart.borrow_mut() = Some(Box::new(callback));
    }

    /// Update the list of charted signals
    pub fn set_charted_signals(&self, signals: Vec<String>) {
        *self.charted_signals.borrow_mut() = signals;
    }

    /// Check if there's a pending chart toggle request and return the signal name
    pub fn take_chart_toggle_request(&self) -> Option<String> {
        self.chart_toggle_request.borrow_mut().take()
    }

    /// Request to toggle a signal on the chart
    fn request_chart_toggle(&self, signal_name: String) {
        // Include bus in the signal key for bus-aware tracking
        let bus_id = self.selected_bus.unwrap_or(0);
        let key = format!("{}@bus{}", signal_name, bus_id);
        *self.chart_toggle_request.borrow_mut() = Some(key);
    }

    /// Get the currently selected (message_id, bus)
    pub fn get_selected(&self) -> Option<(u32, u8)> {
        match (self.selected_message_id, self.selected_bus) {
            (Some(id), Some(bus)) => Some((id, bus)),
            _ => None,
        }
    }

    pub fn set_message(&mut self, id: u32, bus: u8, data: &[u8]) {
        // Only update if this is the currently selected (id, bus) combination
        if self.selected_message_id == Some(id) && self.selected_bus == Some(bus) {
            let old_data = self.last_data;
            let mut padded_new: [u8; 8] = [0; 8];
            for (i, &byte) in data.iter().enumerate() {
                if i < 8 {
                    padded_new[i] = byte;
                }
            }
            self.update_activity(&old_data, &padded_new);
        }

        self.selected_message_id = Some(id);
        self.last_data = self.current_data;
        self.current_data = [0; 8];
        for (i, &byte) in data.iter().enumerate() {
            if i < 8 {
                self.current_data[i] = byte;
            }
        }
    }

    fn update_activity(&mut self, old_data: &[u8; 8], new_data: &[u8; 8]) {
        for byte_idx in 0..8 {
            let changed = old_data[byte_idx] ^ new_data[byte_idx];
            for bit_idx in 0..8 {
                if (changed >> bit_idx) & 1 == 1 {
                    // Use same bit numbering as the visualizer (reversed within each byte)
                    let abs_bit = byte_idx * 8 + (7 - bit_idx);
                    self.bit_flip_counts[abs_bit] += 1;
                    self.max_flip_count = self.max_flip_count.max(self.bit_flip_counts[abs_bit]);
                }
            }
        }
    }

    pub fn reset_activity(&mut self) {
        self.bit_flip_counts = [0; 64];
        self.max_flip_count = 0;
    }

    pub fn clear(&mut self) {
        self.selected_message_id = None;
        self.selected_bus = None;
        self.current_data = [0; 8];
        self.selection_start = None;
        self.selection_end = None;
        self.is_dragging = false;
    }

    pub fn render(&mut self, ui: &Ui, dbc: &mut DbcFile, is_open: &mut bool) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/can-viz-chart-debug.txt")
            .ok();
        if let Some(ref mut f) = f {
            let _ = writeln!(f, "Bit Visualizer render called, is_open={}", *is_open);
            let _ = writeln!(f, "  selected_message_id={:?}", self.selected_message_id);
        }

        ui.window("Bit Visualizer")
            .size([650.0, 550.0], Condition::FirstUseEver)
            .position([100.0, 100.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui, dbc);
            });

        if self.show_create_dialog {
            self.render_create_dialog(ui, dbc);
        }

        if self.show_edit_dialog {
            self.render_edit_dialog(ui, dbc);
        }
    }

    fn render_content(&mut self, ui: &Ui, dbc: &mut DbcFile) {
        if let Some(id) = self.selected_message_id {
            ui.text(format!("Message ID: 0x{:03X} ({})", id, id));
            if let Some(msg_def) = dbc.get_message(id) {
                ui.same_line();
                ui.text_colored([0.5, 0.8, 0.5, 1.0], &format!("({})", msg_def.name));
            }
        } else {
            ui.text_colored([0.6, 0.6, 0.6, 1.0], "No message selected");
            ui.text("Select a message from the Messages list to visualize its bits");
            return;
        }

        ui.separator();

        ui.checkbox("Show Signal Colors", &mut self.show_signals);
        ui.same_line();
        if ui.small_button("Reset Activity") {
            self.reset_activity();
        }

        ui.separator();

        self.render_bit_grid(ui, dbc);
        ui.separator();
        self.render_decoded_signals(ui, dbc);
    }

    fn render_bit_grid(&mut self, ui: &Ui, dbc: &DbcFile) {
        let signals = self.get_signal_info(dbc);
        let mut bit_rects: Vec<(usize, [f32; 2], [f32; 2])> = Vec::new();

        // For header alignment, capture positions from first row of buttons
        let mut header_positions: Vec<[f32; 2]> = Vec::new();

        ui.separator();

        // Header row with bit positions (now empty, using spaces to preserve layout)
        ui.text("     ");
        ui.same_line();
        for i in 0..8 {
            ui.text("  ");
            if i < 7 {
                ui.same_line();
            }
        }

        let selection_bits = self.get_selection_bits();

        for byte_idx in 0..8 {
            let byte_val = self.current_data[byte_idx];

            ui.text(format!("Byte {}: ", byte_idx));
            ui.same_line();

            for bit_idx in (0..8).rev() {
                let bit_val = (byte_val >> bit_idx) & 1;
                let abs_bit_pos = byte_idx * 8 + (7 - bit_idx);

                let (mut bg_color, signal_name, is_msb, is_lsb) = if self.show_signals {
                    self.get_bit_signal_info(abs_bit_pos, &signals)
                } else {
                    ([0.3, 0.3, 0.3, 1.0], None, false, false)
                };

                // Apply activity overlay only when NOT showing signal colors
                if !self.show_signals {
                    let activity = self.get_bit_activity(abs_bit_pos);
                    if activity > 0.0 {
                        bg_color[0] = (bg_color[0] + activity * 0.4).min(1.0);
                        bg_color[1] = (bg_color[1] + activity * 0.2).min(1.0);
                    }
                }

                let is_selected = selection_bits.contains(&abs_bit_pos);

                // Pad with space to make all buttons 2 chars wide (matching M/L indicators)
                let indicator = if is_msb { "M" } else if is_lsb { "L" } else { " " };
                let button_label = format!("{}{}##b{}", bit_val, indicator, abs_bit_pos);

                let _color_token = ui.push_style_color(StyleColor::Button, bg_color);
                let _hover_token = ui.push_style_color(StyleColor::ButtonHovered, [
                    (bg_color[0] + 0.2).min(1.0),
                    (bg_color[1] + 0.2).min(1.0),
                    (bg_color[2] + 0.2).min(1.0),
                    1.0,
                ]);
                let _active_token = ui.push_style_color(StyleColor::ButtonActive, [
                    (bg_color[0] + 0.3).min(1.0),
                    (bg_color[1] + 0.3).min(1.0),
                    (bg_color[2] + 0.3).min(1.0),
                    1.0,
                ]);

                ui.small_button(&button_label);

                let min = ui.item_rect_min();
                let max = [min[0] + ui.item_rect_size()[0], min[1] + ui.item_rect_size()[1]];
                bit_rects.push((abs_bit_pos, min, max));

                // Capture button positions from first row for header alignment
                if byte_idx == 0 && header_positions.len() < 8 {
                    header_positions.push([(min[0] + max[0]) / 2.0, min[1]]);
                }

                if is_selected {
                    let draw_list = ui.get_window_draw_list();
                    draw_list.add_rect(min, max, [1.0, 1.0, 0.0, 1.0]).thickness(2.0).build();
                }

                if ui.is_item_hovered() {
                    if ui.is_mouse_clicked(imgui::MouseButton::Left) {
                        self.selection_start = Some(abs_bit_pos);
                        self.selection_end = Some(abs_bit_pos);
                        self.is_dragging = true;
                    }

                    ui.tooltip(|| {
                        ui.text(format!("Bit {} (byte {}, bit {})", abs_bit_pos, byte_idx, bit_idx));
                        ui.text(format!("Value: {}", bit_val));
                        if let Some(ref name) = signal_name {
                            ui.separator();
                            ui.text_colored([0.5, 0.8, 1.0, 1.0], format!("Signal: {}", name));
                            if is_msb { ui.text_colored([0.9, 0.9, 0.5, 1.0], "(MSB)"); }
                            if is_lsb { ui.text_colored([0.9, 0.9, 0.5, 1.0], "(LSB)"); }
                        }
                        let activity = self.get_bit_activity(abs_bit_pos);
                        if activity > 0.0 {
                            ui.text_colored([1.0, 0.7, 0.4, 1.0], format!("Activity: {:.0}%", activity * 100.0));
                        }
                    });
                }

                if bit_idx > 0 {
                    ui.same_line();
                }
            }

            ui.same_line();
            ui.text_colored([0.6, 0.6, 0.6, 1.0], format!(" 0x{:02X}", byte_val));

            // After rendering first byte, draw the header numbers above using captured positions
            if byte_idx == 0 && !header_positions.is_empty() {
                let draw_list = ui.get_window_draw_list();
                for (i, pos) in header_positions.iter().enumerate() {
                    let bit = 7 - i;  // Reverse: 7, 6, 5, ..., 0
                    let text = format!("{}", bit);
                    let text_width = ui.calc_text_size(&text)[0];
                    // Draw above the button (subtract line height + spacing)
                    let text_y = pos[1] - ui.text_line_height_with_spacing();
                    draw_list.add_text([pos[0] - text_width / 2.0, text_y], [0.7, 0.7, 0.7, 1.0], text);
                }
            }
        }

        if self.is_dragging {
            let mouse_pos = ui.io().mouse_pos;

            for (abs_bit, min, max) in &bit_rects {
                if mouse_pos[0] >= min[0] && mouse_pos[0] <= max[0] &&
                   mouse_pos[1] >= min[1] && mouse_pos[1] <= max[1] {
                    self.selection_end = Some(*abs_bit);
                    break;
                }
            }

            if ui.is_mouse_released(imgui::MouseButton::Left) {
                self.is_dragging = false;
                if self.selection_start.is_some() && self.selection_end.is_some() {
                    self.open_create_dialog();
                }
            }
        }

        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            if !self.is_dragging {
                let (min_bit, max_bit) = if start <= end { (start, end) } else { (end, start) };
                let bit_count = max_bit - min_bit + 1;
                ui.text_colored([1.0, 1.0, 0.0, 1.0], format!("Selected: bits {}-{} ({} bits)", min_bit, max_bit, bit_count));
                ui.same_line();
                if ui.small_button("Clear") {
                    self.clear_selection();
                }
            }
        }
    }

    fn get_bit_activity(&self, bit_pos: usize) -> f32 {
        if self.max_flip_count == 0 { return 0.0; }
        let count = self.bit_flip_counts[bit_pos];
        if count == 0 { 0.0 } else { (count as f32 / self.max_flip_count as f32).sqrt() }
    }

    fn get_selection_bits(&self) -> Vec<usize> {
        match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) => {
                let (min_bit, max_bit) = if start <= end { (start, end) } else { (end, start) };
                (min_bit..=max_bit).collect()
            }
            _ => Vec::new(),
        }
    }

    fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }

    fn open_create_dialog(&mut self) {
        self.signal_counter += 1;
        self.new_signal_name = format!("NEW_SIGNAL_{}", self.signal_counter);
        self.new_signal_factor = String::from("1");
        self.new_signal_offset = String::from("0");
        self.new_signal_unit = String::new();
        self.show_create_dialog = true;
    }

    fn open_edit_dialog(&mut self, signal_idx: usize, signal: &DbcSignal) {
        self.editing_signal_idx = Some(signal_idx);
        self.editing_signal_name = signal.name.clone();
        self.edit_start_bit = signal.start_bit;
        self.edit_bit_length = signal.bit_length;
        self.edit_is_signed = signal.value_type == ValueType::Signed;
        self.edit_is_little_endian = signal.byte_order == ByteOrder::Intel;
        self.edit_factor = signal.factor.to_string();
        self.edit_offset = signal.offset.to_string();
        self.edit_unit = signal.unit.clone().unwrap_or_default();
        self.show_edit_dialog = true;
    }

    fn render_create_dialog(&mut self, ui: &Ui, dbc: &mut DbcFile) {
        if !self.show_create_dialog { return; }

        let (start_bit, bit_length) = if let (Some(s), Some(e)) = (self.selection_start, self.selection_end) {
            let (min, max) = if s <= e { (s, e) } else { (e, s) };
            (min as u8, (max - min + 1) as u8)
        } else {
            (0, 1)
        };

        let mut dialog_open = self.show_create_dialog;
        let mut name = self.new_signal_name.clone();
        let mut is_little_endian = self.new_signal_is_little_endian;
        let mut is_signed = self.new_signal_is_signed;
        let mut factor = self.new_signal_factor.clone();
        let mut offset = self.new_signal_offset.clone();
        let mut unit = self.new_signal_unit.clone();

        let mut should_create = false;
        let mut should_cancel = false;

        ui.window("Create Signal")
            .size([400.0, 320.0], Condition::FirstUseEver)
            .position([200.0, 200.0], Condition::FirstUseEver)
            .opened(&mut dialog_open)
            .build(|| {
                ui.text("Create New Signal");
                ui.separator();

                ui.text("Name:"); ui.same_line();
                ui.input_text("##name", &mut name).build();

                ui.text(format!("Start bit: {}  |  Length: {} bits", start_bit, bit_length));
                ui.separator();

                ui.text("Byte order:");
                if ui.selectable_config(format!("Intel (little-endian){}", if is_little_endian { " *" } else { "" }))
                    .selected(is_little_endian).build() { is_little_endian = true; }
                if ui.selectable_config(format!("Motorola (big-endian){}", if !is_little_endian { " *" } else { "" }))
                    .selected(!is_little_endian).build() { is_little_endian = false; }

                ui.separator();

                ui.text("Value type:");
                if ui.selectable_config(format!("Unsigned{}", if !is_signed { " *" } else { "" }))
                    .selected(!is_signed).build() { is_signed = false; }
                if ui.selectable_config(format!("Signed{}", if is_signed { " *" } else { "" }))
                    .selected(is_signed).build() { is_signed = true; }

                ui.separator();

                ui.text("Factor:"); ui.same_line();
                ui.input_text("##factor", &mut factor).build();
                ui.text("Offset:"); ui.same_line();
                ui.input_text("##offset", &mut offset).build();
                ui.text("Unit:"); ui.same_line();
                ui.input_text("##unit", &mut unit).build();

                ui.separator();

                if ui.button("Create") { should_create = true; }
                ui.same_line();
                if ui.button("Cancel") { should_cancel = true; }
            });

        self.new_signal_name = name;
        self.new_signal_is_little_endian = is_little_endian;
        self.new_signal_is_signed = is_signed;
        self.new_signal_factor = factor;
        self.new_signal_offset = offset;
        self.new_signal_unit = unit;

        if should_cancel || !dialog_open {
            self.show_create_dialog = false;
            self.clear_selection();
        } else if should_create {
            if let Some(msg_id) = self.selected_message_id {
                if let Ok(factor_val) = self.new_signal_factor.parse::<f64>() {
                    if let Ok(offset_val) = self.new_signal_offset.parse::<f64>() {
                        let signal = DbcSignal {
                            name: self.new_signal_name.clone(),
                            start_bit,
                            bit_length,
                            byte_order: if self.new_signal_is_little_endian { ByteOrder::Intel } else { ByteOrder::Motorola },
                            value_type: if self.new_signal_is_signed { ValueType::Signed } else { ValueType::Unsigned },
                            factor: factor_val,
                            offset: offset_val,
                            minimum: None,
                            maximum: None,
                            unit: if self.new_signal_unit.is_empty() { None } else { Some(self.new_signal_unit.clone()) },
                            multiplexor: None,
                        };

                        if dbc.get_message(msg_id).is_none() {
                            let msg_name = format!("MSG_{:03X}", msg_id);
                            dbc.add_message(DbcMessage::new(msg_id, &msg_name, 8));
                        }

                        if let Some(msg) = dbc.get_message_mut(msg_id) {
                            msg.add_signal(signal.clone());
                        }

                        if let Some(ref mut callback) = *self.on_signal_created.borrow_mut() {
                            callback(msg_id, signal);
                        }
                    }
                }
            }
            self.show_create_dialog = false;
            self.clear_selection();
        }

        self.show_create_dialog = dialog_open && !should_cancel && !should_create;
    }

    fn render_edit_dialog(&mut self, ui: &Ui, dbc: &mut DbcFile) {
        if !self.show_edit_dialog { return; }

        let mut dialog_open = self.show_edit_dialog;
        let mut name = self.editing_signal_name.clone();
        let mut start_bit = self.edit_start_bit;
        let mut bit_length = self.edit_bit_length;
        let mut is_little_endian = self.edit_is_little_endian;
        let mut is_signed = self.edit_is_signed;
        let mut factor = self.edit_factor.clone();
        let mut offset = self.edit_offset.clone();
        let mut unit = self.edit_unit.clone();

        let mut should_save = false;
        let mut should_cancel = false;
        let mut should_delete = false;

        ui.window("Edit Signal")
            .size([400.0, 380.0], Condition::FirstUseEver)
            .position([200.0, 200.0], Condition::FirstUseEver)
            .opened(&mut dialog_open)
            .build(|| {
                ui.text("Edit Signal");
                ui.separator();

                ui.text("Name:"); ui.same_line();
                ui.input_text("##name", &mut name).build();

                ui.separator();

                // Editable bit position - using input_text and parsing
                ui.text("Start bit:"); ui.same_line();
                let mut start_str = start_bit.to_string();
                ui.input_text("##startbit", &mut start_str).build();
                if let Ok(v) = start_str.parse::<u8>() {
                    start_bit = v.min(63);
                }
                ui.text("Bit length:"); ui.same_line();
                let mut len_str = bit_length.to_string();
                ui.input_text("##bitlen", &mut len_str).build();
                if let Ok(v) = len_str.parse::<u8>() {
                    bit_length = v.max(1).min(64);
                }

                ui.separator();

                ui.text("Byte order:");
                if ui.selectable_config(format!("Intel (little-endian){}", if is_little_endian { " *" } else { "" }))
                    .selected(is_little_endian).build() { is_little_endian = true; }
                if ui.selectable_config(format!("Motorola (big-endian){}", if !is_little_endian { " *" } else { "" }))
                    .selected(!is_little_endian).build() { is_little_endian = false; }

                ui.separator();

                ui.text("Value type:");
                if ui.selectable_config(format!("Unsigned{}", if !is_signed { " *" } else { "" }))
                    .selected(!is_signed).build() { is_signed = false; }
                if ui.selectable_config(format!("Signed{}", if is_signed { " *" } else { "" }))
                    .selected(is_signed).build() { is_signed = true; }

                ui.separator();

                ui.text("Factor:"); ui.same_line();
                ui.input_text("##factor", &mut factor).build();
                ui.text("Offset:"); ui.same_line();
                ui.input_text("##offset", &mut offset).build();
                ui.text("Unit:"); ui.same_line();
                ui.input_text("##unit", &mut unit).build();

                ui.separator();

                if ui.button("Save") { should_save = true; }
                ui.same_line();
                if ui.button("Cancel") { should_cancel = true; }
                ui.same_line();
                ui.text_colored([0.5, 0.5, 0.5, 1.0], "  |  ");
                ui.same_line();
                let _del_color = ui.push_style_color(StyleColor::Button, [0.6, 0.2, 0.2, 1.0]);
                if ui.button("Delete") { should_delete = true; }
            });

        self.editing_signal_name = name;
        self.edit_start_bit = start_bit;
        self.edit_bit_length = bit_length;
        self.edit_is_little_endian = is_little_endian;
        self.edit_is_signed = is_signed;
        self.edit_factor = factor;
        self.edit_offset = offset;
        self.edit_unit = unit;

        if should_cancel || !dialog_open {
            self.show_edit_dialog = false;
            self.editing_signal_idx = None;
        } else if should_delete {
            // Delete the signal
            if let Some(msg_id) = self.selected_message_id {
                if let Some(idx) = self.editing_signal_idx {
                    if let Some(msg) = dbc.get_message_mut(msg_id) {
                        if idx < msg.signals.len() {
                            msg.signals.remove(idx);
                        }
                    }
                }
            }
            self.show_edit_dialog = false;
            self.editing_signal_idx = None;
        } else if should_save {
            // Update the signal
            if let Some(msg_id) = self.selected_message_id {
                if let Some(idx) = self.editing_signal_idx {
                    if let Ok(factor_val) = self.edit_factor.parse::<f64>() {
                        if let Ok(offset_val) = self.edit_offset.parse::<f64>() {
                            if let Some(msg) = dbc.get_message_mut(msg_id) {
                                if idx < msg.signals.len() {
                                    msg.signals[idx].name = self.editing_signal_name.clone();
                                    msg.signals[idx].start_bit = self.edit_start_bit;
                                    msg.signals[idx].bit_length = self.edit_bit_length;
                                    msg.signals[idx].byte_order = if self.edit_is_little_endian { ByteOrder::Intel } else { ByteOrder::Motorola };
                                    msg.signals[idx].value_type = if self.edit_is_signed { ValueType::Signed } else { ValueType::Unsigned };
                                    msg.signals[idx].factor = factor_val;
                                    msg.signals[idx].offset = offset_val;
                                    msg.signals[idx].unit = if self.edit_unit.is_empty() { None } else { Some(self.edit_unit.clone()) };
                                }
                            }
                        }
                    }
                }
            }
            self.show_edit_dialog = false;
            self.editing_signal_idx = None;
        }

        self.show_edit_dialog = dialog_open && !should_cancel && !should_save && !should_delete;
    }

    fn get_signal_info(&self, dbc: &DbcFile) -> Vec<SignalInfo> {
        let mut result = Vec::new();

        if let Some(id) = self.selected_message_id {
            if let Some(bus) = self.selected_bus {
                if let Some(msg_def) = dbc.get_message(id) {
                    for (i, signal) in msg_def.signals.iter().enumerate() {
                        // Use hash of signal name for consistent color across messages
                        // This ensures the same signal name always gets the same color
                        let color_idx = Self::hash_color_index(&signal.name);
                        result.push(SignalInfo {
                            name: signal.name.clone(),
                            start_bit: signal.start_bit,
                            bit_length: signal.bit_length,
                            byte_order: signal.byte_order,
                            color_idx,
                            bus_id: bus,  // Include bus in signal info
                        });
                    }
                }
            }
        }

        result
    }

    fn get_bit_signal_info(&self, bit_pos: usize, signals: &[SignalInfo]) -> ([f32; 4], Option<String>, bool, bool) {
        for signal in signals {
            let bits = signal.get_bit_positions();
            if bits.contains(&bit_pos) {
                let color = SIGNAL_COLORS[signal.color_idx];
                let is_msb = bit_pos == signal.get_msb_pos();
                let is_lsb = bit_pos == signal.get_lsb_pos();
                return (color, Some(signal.name.clone()), is_msb, is_lsb);
            }
        }
        ([0.15, 0.15, 0.15, 1.0], None, false, false)
    }

    fn render_decoded_signals(&mut self, ui: &Ui, dbc: &mut DbcFile) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/can-viz-chart-debug.txt")
            .ok();

        ui.text("Decoded Signals:");

        if let Some(id) = self.selected_message_id {
            if let Some(msg_def) = dbc.get_message(id) {
                if let Some(ref mut f) = f {
                    let _ = writeln!(f, "render_decoded_signals: {} signals for msg 0x{:X}", msg_def.signals.len(), id);
                }
                if msg_def.signals.is_empty() {
                    ui.text_colored([0.6, 0.6, 0.6, 1.0], "  No signals defined");
                    ui.text_colored([0.6, 0.6, 0.6, 1.0], "  Click and drag on bits to create one");
                    return;
                }

                // Collect signal data first to avoid borrow issues
                let signal_data: Vec<(String, u8, u8, ByteOrder, ValueType, f64, f64, Option<String>)> =
                    msg_def.signals.iter()
                        .map(|s| (
                            s.name.clone(),
                            s.start_bit,
                            s.bit_length,
                            s.byte_order,
                            s.value_type,
                            s.factor,
                            s.offset,
                            s.unit.clone()
                        ))
                        .collect();

                // Get charted signals for highlighting (clone to avoid borrow issues)
                let charted: Vec<String> = self.charted_signals.borrow().clone();

                // Two columns: Signal name + Value, Chart button
                // Use available width for responsive sizing
                let avail_width = ui.content_region_avail()[0];
                let chart_btn_width = 35.0;
                let signal_col_width = avail_width - chart_btn_width - 10.0; // 10 for padding

                ui.columns(2, "signal_cols", false);
                ui.set_column_width(0, signal_col_width);
                ui.set_column_width(1, chart_btn_width);

                for (i, (name, start_bit, bit_length, byte_order, value_type, factor, offset, unit)) in signal_data.iter().enumerate() {
                    let color = SIGNAL_COLORS[i % SIGNAL_COLORS.len()];

                    // Column 1: Signal name (clickable for edit) + decoded value
                    let _color_token = ui.push_style_color(StyleColor::Button, color);
                    ui.small_button(" ");
                    drop(_color_token);
                    ui.same_line();

                    // Make signal name a selectable item for editing
                    let is_selected = self.editing_signal_idx == Some(i);
                    if ui.selectable_config(&name).selected(is_selected).build() {
                        // Open edit dialog when clicked
                        let signal = DbcSignal {
                            name: name.clone(),
                            start_bit: *start_bit,
                            bit_length: *bit_length,
                            byte_order: *byte_order,
                            value_type: *value_type,
                            factor: *factor,
                            offset: *offset,
                            unit: unit.clone(),
                            minimum: None,
                            maximum: None,
                            multiplexor: None,
                        };
                        self.open_edit_dialog(i, &signal);
                    }

                    // Tooltip with signal details
                    if ui.is_item_hovered() {
                        ui.tooltip(|| {
                            ui.text_colored([0.7, 0.7, 0.7, 1.0], "Click to edit");
                        });
                    }

                    // Show decoded value on same line
                    ui.same_line();

                    if let Some(raw_value) = extract_bits(
                        &self.current_data,
                        *start_bit,
                        *bit_length,
                        *byte_order
                    ) {
                        let raw_value_i64 = if *value_type == ValueType::Signed {
                            sign_extend(raw_value, *bit_length)
                        } else {
                            raw_value as i64
                        };

                        // Look up value description in value_tables
                        let value_desc = dbc.value_tables.get(name)
                            .and_then(|descriptions| {
                                descriptions.iter()
                                    .find(|d| d.value == raw_value_i64)
                                    .map(|d| d.description.clone())
                            });

                        if let Some(desc) = value_desc {
                            // Show the named value (e.g., "Valid")
                            ui.text_colored([0.4, 0.9, 0.4, 1.0], &desc);
                            ui.same_line();
                            ui.text_colored([0.5, 0.5, 0.5, 1.0], format!("({})", raw_value_i64));
                        } else {
                            // Show physical value with unit
                            let physical_value = (raw_value_i64 as f64) * factor + offset;

                            let value_str = if let Some(ref unit) = unit {
                                if unit.is_empty() {
                                    format!("{:.3}", physical_value)
                                } else {
                                    format!("{:.3} {}", physical_value, unit)
                                }
                            } else {
                                format!("{:.3}", physical_value)
                            };

                            ui.text_colored([0.9, 0.9, 0.9, 1.0], &value_str);
                            ui.same_line();
                            ui.text_colored([0.5, 0.5, 0.5, 1.0], format!("({})", raw_value_i64));
                        }
                    } else {
                        ui.text_colored([0.5, 0.5, 0.5, 1.0], "—");
                    }

                    ui.next_column();

                    // Column 2: Chart button
                    let is_charted = charted.contains(name);
                    let btn_color = if is_charted {
                        [0.2, 0.6, 0.3, 0.9]  // Green if charted
                    } else {
                        [0.3, 0.3, 0.4, 0.8]  // Gray if not
                    };

                    let _chart_color = ui.push_style_color(StyleColor::Button, btn_color);
                    // Use simple ASCII characters that render everywhere
                    let btn_label = if is_charted { "+" } else { "+" };
                    if ui.small_button(&format!("{}##chart{}", btn_label, i)) {
                        use std::io::Write;
                        let mut f = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("/tmp/can-viz-chart-debug.txt")
                            .ok();
                        if let Some(ref mut f) = f {
                            let _ = writeln!(f, "Button clicked for signal: {}", name);
                        }
                        self.request_chart_toggle(name.clone());
                    }
                    drop(_chart_color);

                    if ui.is_item_hovered() {
                        ui.tooltip(|| {
                            if is_charted {
                                ui.text("Remove from chart");
                            } else {
                                ui.text("Add to chart");
                            }
                        });
                    }

                    ui.next_column();
                }

                ui.columns(1, "", false);
            } else {
                ui.text_colored([0.6, 0.6, 0.6, 1.0], "  Message not defined in DBC");
            }
        } else {
            ui.text_colored([0.6, 0.6, 0.6, 1.0], "  No message selected");
        }
    }

    /// Generate a consistent color index for a signal name using a simple hash
    /// This ensures the same signal name always gets the same color
    fn hash_color_index(name: &str) -> usize {
        let mut hash: usize = 5381;
        for c in name.bytes() {
            hash = hash.wrapping_mul(33).wrapping_add(c as usize);
        }
        hash % SIGNAL_COLORS.len()
    }
}

fn sign_extend(value: u64, bit_length: u8) -> i64 {
    if bit_length >= 64 { return value as i64; }
    let sign_bit = 1u64 << (bit_length - 1);
    if value & sign_bit != 0 {
        let mask = !((1u64 << bit_length) - 1);
        (value | mask) as i64
    } else {
        value as i64
    }
}

impl Default for BitVisualizerWindow {
    fn default() -> Self {
        Self::new()
    }
}

struct SignalInfo {
    name: String,
    start_bit: u8,
    bit_length: u8,
    byte_order: ByteOrder,
    color_idx: usize,
    bus_id: u8,
}

impl SignalInfo {
    fn get_bit_positions(&self) -> Vec<usize> {
        let start = self.start_bit as usize;
        let end = start + self.bit_length as usize;
        (start..end).collect()
    }

    fn get_msb_pos(&self) -> usize {
        match self.byte_order {
            ByteOrder::Intel => self.start_bit as usize + self.bit_length as usize - 1,
            ByteOrder::Motorola => self.start_bit as usize,
        }
    }

    fn get_lsb_pos(&self) -> usize {
        match self.byte_order {
            ByteOrder::Intel => self.start_bit as usize,
            ByteOrder::Motorola => self.start_bit as usize + self.bit_length as usize - 1,
        }
    }
}
