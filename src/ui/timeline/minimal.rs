//! Minimal/Modern Style timeline - Clean lines with floating time display

use imgui::Ui;
use super::{TimelineAction, TimelineData, TimelineTheme};

/// Minimal/modern style timeline with thin slider and circular thumb
pub struct MinimalTimeline {
    /// Whether controls are currently visible (hover state)
    controls_visible: bool,
    /// Animation timer for smooth transitions
    hover_timer: f32,
    /// Slider thumb radius
    thumb_radius: f32,
    /// Dot pattern for density visualization
    dot_spacing: f32,
    /// Current playback state
    is_playing: bool,
}

impl MinimalTimeline {
    pub fn new() -> Self {
        Self {
            controls_visible: false,
            hover_timer: 0.0,
            thumb_radius: 8.0,
            dot_spacing: 6.0,
            is_playing: false,
        }
    }

    /// Set whether playback is active
    pub fn set_playing(&mut self, playing: bool) {
        self.is_playing = playing;
    }

    /// Draw the main slider track
    fn draw_slider_track(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        let track_y = (pos_min[1] + pos_max[1]) / 2.0;
        let track_height = 4.0;

        // Subtle track background
        draw_list.add_rect(
            [pos_min[0], track_y - track_height / 2.0],
            [pos_max[0], track_y + track_height / 2.0],
            [0.25, 0.25, 0.28, 1.0],
        ).filled(true).rounding(2.0).build();

        // Progress fill
        let thumb_x = pos_min[0] + data.position * (pos_max[0] - pos_min[0]);
        draw_list.add_rect(
            [pos_min[0], track_y - track_height / 2.0],
            [thumb_x, track_y + track_height / 2.0],
            [0.4, 0.6, 0.9, 1.0],
        ).filled(true).rounding(2.0).build();

        // Circular thumb
        let thumb_color = if self.controls_visible {
            [0.5, 0.7, 1.0, 1.0] // Brighter when hovered
        } else {
            [0.4, 0.6, 0.9, 1.0]
        };

        draw_list.add_circle(
            [thumb_x, track_y],
            self.thumb_radius,
            thumb_color,
        ).filled(true).num_segments(20).build();

        // Subtle thumb border
        draw_list.add_circle(
            [thumb_x, track_y],
            self.thumb_radius,
            [0.6, 0.8, 1.0, 0.5],
        ).thickness(1.5).num_segments(20).build();
    }

    /// Draw density as subtle dots with varying opacity
    fn draw_density_dots(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        if data.density.is_empty() {
            return;
        }

        let width = pos_max[0] - pos_min[0];
        let max_density = *data.density.iter().max().unwrap_or(&1) as f32;
        let num_dots = (width / self.dot_spacing) as usize;
        let dot_y = (pos_min[1] + pos_max[1]) / 2.0;
        let dot_radius = 1.5;

        for i in 0..num_dots {
            let t = i as f32 / num_dots as f32;
            let x = pos_min[0] + t * width;

            // Get density at this position
            let density_idx = (t * (data.density.len() - 1) as f32) as usize;
            let density_idx = density_idx.min(data.density.len() - 1);
            let density = data.density[density_idx] as f32;
            let opacity = (density / max_density) * 0.6 + 0.1;

            draw_list.add_circle(
                [x, dot_y + 15.0], // Below the track
                dot_radius,
                [0.6, 0.7, 0.9, opacity],
            ).filled(true).num_segments(6).build();
        }
    }

    /// Draw loop region as subtle highlight
    fn draw_loop_region(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        if let (Some(loop_start), Some(loop_end)) = (data.loop_start, data.loop_end) {
            let width = pos_max[0] - pos_min[0];
            let track_y = (pos_min[1] + pos_max[1]) / 2.0;
            let track_height = 8.0;

            let x1 = pos_min[0] + loop_start * width;
            let x2 = pos_min[0] + loop_end * width;

            // Subtle purple/blue highlight
            draw_list.add_rect(
                [x1, track_y - track_height / 2.0],
                [x2, track_y + track_height / 2.0],
                [0.5, 0.4, 0.7, 0.4],
            ).filled(true).rounding(2.0).build();
        }
    }

    /// Draw markers as small colored dots above the track
    fn draw_markers(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        let width = pos_max[0] - pos_min[0];
        let track_y = (pos_min[1] + pos_max[1]) / 2.0;

        for marker in &data.markers {
            let x = pos_min[0] + marker.position * width;

            // Small dot above track
            draw_list.add_circle(
                [x, track_y - 12.0],
                3.0,
                marker.color,
            ).filled(true).num_segments(10).build();

            // Label only visible on hover
            if self.controls_visible {
                draw_list.add_text(
                    [x - marker.label.len() as f32 * 3.0, track_y - 28.0],
                    [marker.color[0], marker.color[1], marker.color[2], 0.7],
                    &marker.label,
                );
            }
        }
    }

    /// Draw floating time display
    fn draw_time_display(
        &self,
        draw_list: &imgui::DrawListMut,
        data: &TimelineData,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        if let (Some(current_time), Some(start_time)) = (data.current_time(), data.start_time) {
            // Calculate relative time in seconds
            let elapsed = (current_time - start_time).num_milliseconds() as f64 / 1000.0;
            let time_str = format!("{:.1}s", elapsed);
            let thumb_x = pos_min[0] + data.position * (pos_max[0] - pos_min[0]);

            // Floating time above thumb
            let text_y = pos_min[1] - 5.0;

            // Subtle background
            let text_width = time_str.len() as f32 * 7.0;
            draw_list.add_rect(
                [thumb_x - text_width / 2.0 - 4.0, text_y - 2.0],
                [thumb_x + text_width / 2.0 + 4.0, text_y + 16.0],
                [0.15, 0.15, 0.18, 0.8],
            ).filled(true).rounding(4.0).build();

            draw_list.add_text(
                [thumb_x - text_width / 2.0, text_y],
                [0.9, 0.9, 0.95, 1.0],
                time_str,
            );
        }
    }

    /// Draw percentage labels at ends
    fn draw_percentage_labels(
        &self,
        draw_list: &imgui::DrawListMut,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
    ) {
        let track_y = (pos_min[1] + pos_max[1]) / 2.0;

        // Left: 0%
        draw_list.add_text(
            [pos_min[0], track_y + 25.0],
            [0.5, 0.5, 0.55, 0.6],
            "0%",
        );

        // Right: 100%
        draw_list.add_text(
            [pos_max[0] - 30.0, track_y + 25.0],
            [0.5, 0.5, 0.55, 0.6],
            "100%",
        );
    }
}

impl Default for MinimalTimeline {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineTheme for MinimalTimeline {
    fn name(&self) -> &'static str {
        "Minimal"
    }

    fn render(&mut self, ui: &Ui, data: &mut TimelineData) -> TimelineAction {
        let mut action = TimelineAction::None;

        // Playback controls on the left
        let playback_btn_width = 30.0;
        let playback_margin = 8.0;

        // Step back button
        if ui.small_button("|<") {
            action = TimelineAction::StepBack;
        }
        ui.same_line();

        // Play/Pause button
        if self.is_playing {
            if ui.small_button("||") {
                action = TimelineAction::Pause;
            }
        } else {
            if ui.small_button(">") {
                action = TimelineAction::Play;
            }
        }
        ui.same_line();

        // Step forward button
        if ui.small_button(">|") {
            action = TimelineAction::StepForward;
        }
        ui.same_line();

        // Slider area - compact height
        let slider_height = 50.0;
        let control_height = if self.controls_visible { 30.0 } else { 0.0 };
        let size = [ui.content_region_avail()[0], slider_height + control_height];
        let draw_list = ui.get_window_draw_list();
        let cursor_pos = ui.cursor_screen_pos();
        let pos_min = cursor_pos;
        let pos_max = [cursor_pos[0] + size[0], cursor_pos[1] + slider_height - 10.0];

        // Track the entire widget area for hover detection
        let widget_area = [
            pos_min[0],
            pos_min[1],
            pos_max[0],
            pos_max[1] + 20.0,
        ];

        // Mouse interaction
        let mouse_pos = ui.io().mouse_pos;
        let is_hovered = mouse_pos[0] >= widget_area[0] && mouse_pos[0] <= widget_area[2] &&
                         mouse_pos[1] >= widget_area[1] && mouse_pos[1] <= widget_area[3];

        // Update hover state with smooth transition
        if is_hovered {
            self.controls_visible = true;
            self.hover_timer = 1.0;
        } else if self.hover_timer > 0.0 {
            self.hover_timer -= 0.05;
            if self.hover_timer <= 0.0 {
                self.controls_visible = false;
            }
        }

        // Draw components
        self.draw_density_dots(&draw_list, data, pos_min, pos_max);
        self.draw_loop_region(&draw_list, data, pos_min, pos_max);
        self.draw_slider_track(&draw_list, data, pos_min, pos_max);
        self.draw_markers(&draw_list, data, pos_min, pos_max);
        self.draw_time_display(&draw_list, data, pos_min, pos_max);
        self.draw_percentage_labels(&draw_list, pos_min, pos_max);

        // Track interaction area
        let track_y = (pos_min[1] + pos_max[1]) / 2.0;
        let track_area = [
            pos_min[0],
            track_y - 15.0,
            pos_max[0],
            track_y + 25.0,
        ];

        let is_in_track = mouse_pos[0] >= track_area[0] && mouse_pos[0] <= track_area[2] &&
                          mouse_pos[1] >= track_area[1] && mouse_pos[1] <= track_area[3];

        if is_in_track {
            let width = pos_max[0] - pos_min[0];
            let rel_x = (mouse_pos[0] - pos_min[0]) / width;

            // Time tooltip on hover
            if let (Some(time), Some(start_time)) = (data.time_at_position(rel_x), data.start_time) {
                let elapsed = (time - start_time).num_milliseconds() as f64 / 1000.0;
                ui.tooltip(|| {
                    ui.text_colored([0.7, 0.8, 0.9, 1.0], format!("Seek to: {:.1}s", elapsed));
                    ui.text_colored([0.5, 0.6, 0.7, 1.0], format!("Position: {:.1}%", rel_x * 100.0));
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
            let width = pos_max[0] - pos_min[0];
            let rel_x = (mouse_pos[0] - pos_min[0]) / width;
            data.position = rel_x.clamp(0.0, 1.0);
            action = TimelineAction::Seek(data.position);
        }

        if ui.is_mouse_released(imgui::MouseButton::Left) {
            data.dragging = false;
        }

        ui.dummy([size[0], slider_height - 10.0]);

        action
    }
}
