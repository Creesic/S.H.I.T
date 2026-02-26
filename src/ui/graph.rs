use imgui::{Condition, StyleColor, Ui};
use chrono::{DateTime, Utc, Duration};

/// Widget for plotting signal values over time
pub struct GraphWidget {
    data_points: Vec<f64>,
    timestamps: Vec<DateTime<Utc>>,
    max_points: usize,
    /// Time window duration in seconds (zoom level)
    time_window_secs: f32,
}

impl GraphWidget {
    pub fn new(max_points: usize) -> Self {
        Self {
            data_points: Vec::with_capacity(max_points),
            timestamps: Vec::with_capacity(max_points),
            max_points,
            time_window_secs: 5.0, // Default 5 second window
        }
    }

    /// Add a data point to the graph
    pub fn add_point(&mut self, value: f64, timestamp: DateTime<Utc>) {
        self.data_points.push(value);
        self.timestamps.push(timestamp);

        // Only trim if we exceed max capacity significantly
        if self.data_points.len() > self.max_points * 2 {
            let trim_count = self.data_points.len() - self.max_points;
            self.data_points.drain(0..trim_count);
            self.timestamps.drain(0..trim_count);
        }
    }

    /// Clear all data points
    pub fn clear(&mut self) {
        self.data_points.clear();
        self.timestamps.clear();
    }

    /// Set the time window duration in seconds
    pub fn set_time_window(&mut self, secs: f32) {
        self.time_window_secs = secs.clamp(0.1, 60.0);
    }

    /// Get the current time window duration
    pub fn time_window(&self) -> f32 {
        self.time_window_secs
    }

    /// Render the graph widget with a current time reference
    pub fn render(&mut self, ui: &Ui, label: &str, current_time: Option<DateTime<Utc>>) {
        if self.data_points.is_empty() {
            ui.text(format!("{}: No data", label));
            return;
        }

        // Time window controls
        ui.text("Time Window:");
        ui.same_line();
        if ui.small_button("-") {
            self.time_window_secs = (self.time_window_secs - 1.0).max(0.5);
        }
        ui.same_line();
        ui.text(format!("{:.1}s", self.time_window_secs));
        ui.same_line();
        if ui.small_button("+") {
            self.time_window_secs = (self.time_window_secs + 1.0).min(60.0);
        }

        let size = [ui.content_region_avail()[0], 200.0];
        let draw_list = ui.get_window_draw_list();
        let cursor_pos = ui.cursor_screen_pos();
        let pos_min = cursor_pos;
        let pos_max = [cursor_pos[0] + size[0], cursor_pos[1] + size[1]];

        // Draw background
        draw_list.add_rect(
            pos_min,
            pos_max,
            ui.style_color(StyleColor::FrameBg),
        ).filled(true).build();

        // Determine time window
        let window_duration = Duration::seconds(self.time_window_secs as i64);
        let (time_start, time_end) = if let Some(ct) = current_time {
            (ct - window_duration / 2, ct + window_duration / 2)
        } else if let (Some(&first), Some(&last)) = (self.timestamps.first(), self.timestamps.last()) {
            (first, last)
        } else {
            return;
        };

        // Find indices within time window
        let window_indices: Vec<usize> = self.timestamps.iter()
            .enumerate()
            .filter(|(_, &ts)| ts >= time_start && ts <= time_end)
            .map(|(i, _)| i)
            .collect();

        if window_indices.len() < 2 {
            ui.dummy(size);
            ui.text("No data in time window");
            return;
        }

        // Calculate value range for visible data
        let (min_val, max_val) = window_indices.iter()
            .filter_map(|&i| self.data_points.get(i))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), &val| {
                (min.min(val), max.max(val))
            });

        // Add some padding to value range
        let val_padding = (max_val - min_val) * 0.1;
        let min_val = min_val - val_padding;
        let max_val = max_val + val_padding;

        // Draw zero line if in range
        if min_val <= 0.0 && max_val >= 0.0 {
            let y_zero = self.value_to_y(0.0, min_val, max_val, pos_min, pos_max);
            draw_list.add_line(
                [pos_min[0], y_zero],
                [pos_max[0], y_zero],
                [0.5, 0.5, 0.5, 0.3],
            ).build();
        }

        // Draw signal line for visible window
        for i in 0..window_indices.len() - 1 {
            let idx1 = window_indices[i];
            let idx2 = window_indices[i + 1];

            let x1 = self.time_to_x(self.timestamps[idx1], time_start, time_end, pos_min, pos_max);
            let y1 = self.value_to_y(self.data_points[idx1], min_val, max_val, pos_min, pos_max);
            let x2 = self.time_to_x(self.timestamps[idx2], time_start, time_end, pos_min, pos_max);
            let y2 = self.value_to_y(self.data_points[idx2], min_val, max_val, pos_min, pos_max);

            draw_list.add_line(
                [x1, y1],
                [x2, y2],
                ui.style_color(StyleColor::PlotLines),
            ).thickness(2.0).build();
        }

        // Draw current time indicator (vertical line at center if we have a current time)
        if current_time.is_some() && time_start <= current_time.unwrap() && time_end >= current_time.unwrap() {
            let x_center = (pos_min[0] + pos_max[0]) / 2.0;
            draw_list.add_line(
                [x_center, pos_min[1]],
                [x_center, pos_max[1]],
                [1.0, 1.0, 0.5, 0.5],
            ).thickness(1.0).build();
        }

        // Draw time labels at edges
        draw_list.add_text(
            [pos_min[0] + 5.0, pos_max[1] - 15.0],
            [0.6, 0.6, 0.6, 0.8],
            format!("-{:.1}s", self.time_window_secs / 2.0),
        );
        draw_list.add_text(
            [pos_max[0] - 35.0, pos_max[1] - 15.0],
            [0.6, 0.6, 0.6, 0.8],
            format!("+{:.1}s", self.time_window_secs / 2.0),
        );

        ui.dummy(size);
        let current = self.data_points.last().copied().unwrap_or(0.0);
        ui.text(format!("{}: {:.2}", label, current));
        ui.text(format!("Range: [{:.2}, {:.2}]", min_val, max_val));
    }

    fn value_to_y(&self, value: f64, min: f64, max: f64, pos_min: [f32; 2], pos_max: [f32; 2]) -> f32 {
        let range = max - min;
        if range == 0.0 {
            return (pos_min[1] + pos_max[1]) / 2.0;
        }
        let normalized = (value - min) / range;
        let clamped = normalized.clamp(0.0, 1.0);
        pos_max[1] - (clamped as f32) * (pos_max[1] - pos_min[1])
    }

    fn time_to_x(&self, time: DateTime<Utc>, time_start: DateTime<Utc>, time_end: DateTime<Utc>, pos_min: [f32; 2], pos_max: [f32; 2]) -> f32 {
        let total_duration = (time_end - time_start).num_milliseconds() as f64;
        if total_duration <= 0.0 {
            return (pos_min[0] + pos_max[0]) / 2.0;
        }
        let elapsed = (time - time_start).num_milliseconds() as f64;
        let normalized = (elapsed / total_duration).clamp(0.0, 1.0);
        pos_min[0] + (normalized as f32) * (pos_max[0] - pos_min[0])
    }
}

/// Simple signal graph wrapper
pub struct SignalGraph {
    graph: GraphWidget,
    label: String,
}

impl SignalGraph {
    pub fn new(label: String) -> Self {
        Self {
            graph: GraphWidget::new(10000), // Store more points for time windowing
            label,
        }
    }

    pub fn add_point(&mut self, value: f64, timestamp: DateTime<Utc>) {
        self.graph.add_point(value, timestamp);
    }

    pub fn clear(&mut self) {
        self.graph.clear();
    }

    pub fn set_time_window(&mut self, secs: f32) {
        self.graph.set_time_window(secs);
    }

    pub fn time_window(&self) -> f32 {
        self.graph.time_window()
    }

    pub fn render(&mut self, ui: &Ui, is_open: &mut bool, current_time: Option<DateTime<Utc>>) {
        ui.window("Signal Graph")
            .size([450.0, 350.0], Condition::FirstUseEver)
            .position([970.0, 30.0], Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                self.render_content(ui, current_time);
            });
    }

    /// Render content with current time reference
    pub fn render_content(&mut self, ui: &Ui, current_time: Option<DateTime<Utc>>) {
        ui.group(|| {
            ui.text(format!("Signal: {}", self.label));
            self.graph.render(ui, &self.label, current_time);
        });
    }
}
