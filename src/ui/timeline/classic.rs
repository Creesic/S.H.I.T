//! Classic timeline implementation - the original CAN-Viz timeline style

use imgui::{StyleColor, Ui};
use super::{TimelineAction, TimelineData, TimelineTheme};

/// Classic timeline renderer
pub struct ClassicTimeline {
    /// Hover state for smooth interactions
    hovered_region: bool,
}

impl ClassicTimeline {
    pub fn new() -> Self {
        Self {
            hovered_region: false,
        }
    }
}

impl Default for ClassicTimeline {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineTheme for ClassicTimeline {
    fn name(&self) -> &'static str {
        "Classic"
    }

    fn render(&mut self, ui: &Ui, data: &mut TimelineData) -> TimelineAction {
        let mut action = TimelineAction::None;

        // Timeline dimensions
        let timeline_height = 60.0;
        let density_height = 30.0;
        let total_height = timeline_height + density_height + 40.0;

        let size = [ui.content_region_avail()[0], total_height];
        let draw_list = ui.get_window_draw_list();
        let cursor_pos = ui.cursor_screen_pos();
        let pos_min = cursor_pos;
        let pos_max = [cursor_pos[0] + size[0], cursor_pos[1] + size[1]];

        // Background
        draw_list.add_rect(
            pos_min,
            [pos_max[0], pos_min[1] + timeline_height + density_height],
            ui.style_color(StyleColor::FrameBg),
        ).filled(true).rounding(4.0).build();

        // Draw message density
        if !data.density.is_empty() {
            let density_pos_min = [pos_min[0], pos_min[1]];
            let density_pos_max = [pos_max[0], pos_min[1] + density_height];
            self.draw_density(&draw_list, data, density_pos_min, density_pos_max);
        }

        // Timeline track area
        let track_pos_min = [pos_min[0], pos_min[1] + density_height];
        let track_pos_max = [pos_max[0], pos_min[1] + density_height + timeline_height];

        // Draw loop region if set
        if let (Some(loop_start), Some(loop_end)) = (data.loop_start, data.loop_end) {
            let x1 = track_pos_min[0] + loop_start * (track_pos_max[0] - track_pos_min[0]);
            let x2 = track_pos_min[0] + loop_end * (track_pos_max[0] - track_pos_min[0]);
            draw_list.add_rect(
                [x1, track_pos_min[1]],
                [x2, track_pos_max[1]],
                [0.3, 0.5, 0.3, 0.5],
            ).filled(true).build();
        }

        // Draw markers
        for marker in &data.markers {
            let x = track_pos_min[0] + marker.position * (track_pos_max[0] - track_pos_min[0]);
            draw_list.add_line(
                [x, track_pos_min[1]],
                [x, track_pos_max[1]],
                marker.color,
            ).thickness(2.0).build();
        }

        // Draw time ticks
        self.draw_time_ticks(&draw_list, data, track_pos_min, track_pos_max);

        // Draw playhead
        let playhead_x = track_pos_min[0] + data.position * (track_pos_max[0] - track_pos_min[0]);
        draw_list.add_line(
            [playhead_x, track_pos_min[1]],
            [playhead_x, track_pos_max[1]],
            [1.0, 1.0, 1.0, 1.0],
        ).thickness(2.0).build();

        // Draw playhead triangle
        let tri_size = 8.0;
        draw_list.add_triangle(
            [playhead_x - tri_size, track_pos_min[1]],
            [playhead_x + tri_size, track_pos_min[1]],
            [playhead_x, track_pos_min[1] + tri_size],
            [1.0, 1.0, 1.0, 1.0],
        ).filled(true).build();

        // Handle mouse interaction
        let mouse_pos = ui.io().mouse_pos;
        let is_hovered = mouse_pos[0] >= track_pos_min[0] && mouse_pos[0] <= track_pos_max[0] &&
                        mouse_pos[1] >= track_pos_min[1] && mouse_pos[1] <= track_pos_max[1];

        self.hovered_region = is_hovered;

        if is_hovered {
            // Show time tooltip
            let rel_x = (mouse_pos[0] - track_pos_min[0]) / (track_pos_max[0] - track_pos_min[0]);
            if let (Some(time), Some(start_time)) = (data.time_at_position(rel_x), data.start_time) {
                let elapsed = (time - start_time).num_milliseconds() as f64 / 1000.0;
                ui.tooltip(|| {
                    ui.text(format!("Time: {:.1}s", elapsed));
                    ui.text(format!("Position: {:.1}%", rel_x * 100.0));
                });
            }

            // Click to seek
            if ui.is_mouse_clicked(imgui::MouseButton::Left) {
                data.position = rel_x.clamp(0.0, 1.0);
                data.dragging = true;
                action = TimelineAction::Seek(data.position);
            }
        }

        // Drag to scrub
        if data.dragging && ui.is_mouse_down(imgui::MouseButton::Left) {
            let rel_x = (mouse_pos[0] - track_pos_min[0]) / (track_pos_max[0] - track_pos_min[0]);
            data.position = rel_x.clamp(0.0, 1.0);
            action = TimelineAction::Seek(data.position);
        }

        if ui.is_mouse_released(imgui::MouseButton::Left) {
            data.dragging = false;
        }

        // Reserve space
        ui.dummy(size);

        // Controls
        ui.spacing();

        // Zoom controls
        ui.text("Zoom:");
        ui.same_line();
        if ui.small_button("-") {
            data.zoom_out();
            action = TimelineAction::Zoom(data.zoom);
        }
        ui.same_line();
        ui.text(format!("{:.1}x", data.zoom));
        ui.same_line();
        if ui.small_button("+") {
            data.zoom_in();
            action = TimelineAction::Zoom(data.zoom);
        }
        ui.same_line();
        if ui.small_button("Reset") {
            data.zoom = 1.0;
            data.pan = 0.0;
            action = TimelineAction::Zoom(1.0);
        }

        // Loop controls
        ui.same_line();
        ui.text("  Loop:");
        ui.same_line();
        let looping = data.loop_start.is_some();
        if ui.small_button(if looping { "Clear" } else { "Set" }) {
            if looping {
                data.clear_loop_region();
                action = TimelineAction::LoopClear;
            } else {
                // Set loop region around current position
                let loop_size = 0.1;
                data.loop_start = Some((data.position - loop_size / 2.0).max(0.0));
                data.loop_end = Some((data.position + loop_size / 2.0).min(1.0));
                action = TimelineAction::LoopSet(data.loop_start.unwrap(), data.loop_end.unwrap());
            }
        }

        // Position display
        ui.same_line();
        if let (Some(current_time), Some(start_time)) = (data.current_time(), data.start_time) {
            let elapsed = (current_time - start_time).num_milliseconds() as f64 / 1000.0;
            ui.text(format!("Time: {:.1}s", elapsed));
        }

        action
    }
}

impl ClassicTimeline {
    fn draw_density(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        if data.density.is_empty() {
            return;
        }

        let max_density = *data.density.iter().max().unwrap_or(&1) as f32;
        let width = pos_max[0] - pos_min[0];
        let height = pos_max[1] - pos_min[1];
        let bar_width = width / data.density.len() as f32;

        for (i, &density) in data.density.iter().enumerate() {
            let x = pos_min[0] + i as f32 * bar_width;
            let bar_height = (density as f32 / max_density) * height;
            let intensity = density as f32 / max_density;

            draw_list.add_rect(
                [x, pos_max[1] - bar_height],
                [x + bar_width - 1.0, pos_max[1]],
                [0.2 + intensity * 0.5, 0.4 + intensity * 0.4, 0.8, 0.8],
            ).filled(true).build();
        }
    }

    fn draw_time_ticks(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        let width = pos_max[0] - pos_min[0];

        // Draw 10 tick marks
        for i in 0..=10 {
            let x = pos_min[0] + (i as f32 / 10.0) * width;
            let tick_height = if i % 2 == 0 { 8.0 } else { 4.0 };

            draw_list.add_line(
                [x, pos_max[1] - tick_height],
                [x, pos_max[1]],
                [0.5, 0.5, 0.5, 0.8],
            ).build();

            // Time label for major ticks
            if i % 2 == 0 {
                if let (Some(time), Some(start_time)) = (data.time_at_position(i as f32 / 10.0), data.start_time) {
                    let elapsed = (time - start_time).num_milliseconds() as f64 / 1000.0;
                    draw_list.add_text(
                        [x - 15.0, pos_max[1] + 2.0],
                        [0.6, 0.6, 0.6, 0.8],
                        format!("{:.0}s", elapsed),
                    );
                }
            }
        }
    }
}
