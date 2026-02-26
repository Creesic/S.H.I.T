use imgui::{Condition, Ui, Window};
use crate::core::dbc::DbcFile;

/// Window for browsing and selecting signals to plot
pub struct SignalListWindow {
    selected_message: Option<u32>,
    filter_text: String,
    plottable_signals: Vec<PlottableSignal>,
}

/// A signal that can be plotted
#[derive(Clone)]
pub struct PlottableSignal {
    pub message_id: u32,
    pub message_name: String,
    pub signal_name: String,
    pub unit: Option<String>,
    pub is_plotted: bool,
}

impl SignalListWindow {
    pub fn new() -> Self {
        Self {
            selected_message: None,
            filter_text: String::new(),
            plottable_signals: Vec::new(),
        }
    }

    /// Update the signal list from a DBC file
    pub fn update_from_dbc(&mut self, dbc: &DbcFile) {
        self.plottable_signals.clear();

        for msg in &dbc.messages {
            for signal in &msg.signals {
                self.plottable_signals.push(PlottableSignal {
                    message_id: msg.id,
                    message_name: msg.name.clone(),
                    signal_name: signal.name.clone(),
                    unit: signal.unit.clone(),
                    is_plotted: false,
                });
            }
        }

        // Sort by message name, then signal name
        self.plottable_signals.sort_by(|a, b| {
            match a.message_name.cmp(&b.message_name) {
                std::cmp::Ordering::Equal => a.signal_name.cmp(&b.signal_name),
                other => other,
            }
        });
    }

    /// Get list of signals that should be plotted
    pub fn get_plotted_signals(&self) -> Vec<(u32, &str)> {
        self.plottable_signals.iter()
            .filter(|s| s.is_plotted)
            .map(|s| (s.message_id, s.signal_name.as_str()))
            .collect()
    }

    /// Toggle a signal's plot status
    pub fn toggle_plot(&mut self, message_id: u32, signal_name: &str) {
        for signal in &mut self.plottable_signals {
            if signal.message_id == message_id && signal.signal_name == signal_name {
                signal.is_plotted = !signal.is_plotted;
                break;
            }
        }
    }

    /// Set plot status for a signal
    pub fn set_plot(&mut self, message_id: u32, signal_name: &str, plotted: bool) {
        for signal in &mut self.plottable_signals {
            if signal.message_id == message_id && signal.signal_name == signal_name {
                signal.is_plotted = plotted;
                break;
            }
        }
    }

    /// Clear all plotted signals
    pub fn clear_all_plots(&mut self) {
        for signal in &mut self.plottable_signals {
            signal.is_plotted = false;
        }
    }

    /// Render the signal list window
    pub fn render(&mut self, ui: &Ui, is_open: &mut bool) {
        ui.window("Signal Browser")
            .size([300.0, 400.0], Condition::FirstUseEver)
            .position([970.0, 400.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui);
            });
    }

    /// Render content without window wrapper - for embedding in workspace
    pub fn render_content(&mut self, ui: &Ui) {
        // Filter input
        ui.text("Filter:");
        ui.same_line();
        ui.input_text("##filter", &mut self.filter_text)
            .hint("Type to filter...")
            .build();

        ui.separator();

        // Action buttons
        if ui.button("Plot Selected") {
            // Plot all selected signals
        }
        ui.same_line();
        if ui.button("Clear All") {
            self.clear_all_plots();
        }

        ui.separator();

        // Signal list with collapsible message groups
        let filter_lower = self.filter_text.to_lowercase();
        let mut current_msg = String::new();
        let mut msg_open = true;

        for signal in &mut self.plottable_signals {
            // Apply filter
            if !filter_lower.is_empty() {
                let signal_matches = signal.signal_name.to_lowercase().contains(&filter_lower) ||
                                    signal.message_name.to_lowercase().contains(&filter_lower);
                if !signal_matches {
                    continue;
                }
            }

            // New message header?
            if signal.message_name != current_msg {
                current_msg = signal.message_name.clone();
                msg_open = ui.collapsing_header(&format!("{} (0x{:03X})", current_msg, signal.message_id), imgui::TreeNodeFlags::empty());
            }

            if !msg_open {
                continue;
            }

            // Signal checkbox
            ui.indent();
            let mut is_plotted = signal.is_plotted;

            let label = if let Some(ref unit) = signal.unit {
                format!("{} [{}]", signal.signal_name, unit)
            } else {
                signal.signal_name.clone()
            };

            if ui.checkbox(&label, &mut is_plotted) {
                signal.is_plotted = is_plotted;
            }

            // Show signal details on hover
            if ui.is_item_hovered() {
                ui.tooltip(|| {
                    ui.text(format!("Message: {}", signal.message_name));
                    ui.text(format!("Signal: {}", signal.signal_name));
                    if let Some(ref unit) = signal.unit {
                        ui.text(format!("Unit: {}", unit));
                    }
                    ui.text("Click to toggle plotting");
                });
            }
            ui.unindent();
        }

        // Summary
        ui.separator();
        let plotted_count = self.plottable_signals.iter().filter(|s| s.is_plotted).count();
        ui.text(format!("{} signals plotted", plotted_count));
    }
}

/// Statistics about a signal
#[derive(Clone)]
pub struct SignalStats {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub count: usize,
}

impl SignalStats {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            mean: 0.0,
            count: 0,
        }
    }

    pub fn update(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.mean = (self.mean * self.count as f64 + value) / (self.count as f64 + 1.0);
        self.count += 1;
    }
}

/// Window showing signal statistics
pub struct SignalStatsWindow {
    stats: Vec<SignalStats>,
}

impl SignalStatsWindow {
    pub fn new() -> Self {
        Self {
            stats: Vec::new(),
        }
    }

    /// Update stats for a signal
    pub fn update_stats(&mut self, signal_name: &str, value: f64) {
        if let Some(stat) = self.stats.iter_mut().find(|s| s.name == signal_name) {
            stat.update(value);
        } else {
            let mut stat = SignalStats::new(signal_name);
            stat.update(value);
            self.stats.push(stat);
        }
    }

    /// Clear all stats
    pub fn clear(&mut self) {
        self.stats.clear();
    }

    /// Get stats for a signal
    pub fn get_stats(&self, signal_name: &str) -> Option<&SignalStats> {
        self.stats.iter().find(|s| s.name == signal_name)
    }

    /// Render the statistics window
    pub fn render(&self, ui: &Ui, is_open: &mut bool) {
        ui.window("Signal Statistics")
            .size([350.0, 250.0], Condition::FirstUseEver)
            .position([1280.0, 30.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui);
            });
    }

    /// Render content without window wrapper - for embedding in workspace
    pub fn render_content(&self, ui: &Ui) {
        if self.stats.is_empty() {
            ui.text("No signal data collected yet.");
            ui.text("Load a CAN log and play it to collect statistics.");
            return;
        }

        // Table header
        ui.columns(5, "stats_table", true);
        ui.text("Signal"); ui.next_column();
        ui.text("Min"); ui.next_column();
        ui.text("Max"); ui.next_column();
        ui.text("Mean"); ui.next_column();
        ui.text("Count"); ui.next_column();
        ui.separator();

        // Table rows
        for stat in &self.stats {
            ui.text(&stat.name); ui.next_column();
            ui.text(format!("{:.2}", stat.min)); ui.next_column();
            ui.text(format!("{:.2}", stat.max)); ui.next_column();
            ui.text(format!("{:.2}", stat.mean)); ui.next_column();
            ui.text(format!("{}", stat.count)); ui.next_column();
        }

        ui.columns(1, "", false);
    }
}
