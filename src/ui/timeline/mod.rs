//! Timeline widget module with multiple visual variants
//!
//! This module provides timeline scrubber widgets for navigating through CAN log data.
//! Multiple visual styles are available to suit different workflows.

mod classic;
mod minimal;

use imgui::Ui;
use chrono::{DateTime, Utc};

pub use classic::ClassicTimeline;
pub use minimal::MinimalTimeline;

/// A marker on the timeline
#[derive(Clone, Debug)]
pub struct TimelineMarker {
    pub position: f32,
    pub label: String,
    pub color: [f32; 4],
}

impl TimelineMarker {
    pub fn new(position: f32, label: &str, color: [f32; 4]) -> Self {
        Self {
            position: position.clamp(0.0, 1.0),
            label: label.to_string(),
            color,
        }
    }
}

/// Shared data for timeline rendering
#[derive(Clone, Debug)]
pub struct TimelineData {
    /// Current playback position (0.0 to 1.0)
    pub position: f32,
    /// Zoom level (1.0 = full view)
    pub zoom: f32,
    /// Pan offset for zoomed view (0.0 to 1.0)
    pub pan: f32,
    /// Whether we're currently dragging
    pub dragging: bool,
    /// Loop region start (0.0 to 1.0), None = no loop
    pub loop_start: Option<f32>,
    /// Loop region end (0.0 to 1.0)
    pub loop_end: Option<f32>,
    /// Message density data for visualization
    pub density: Vec<u32>,
    /// Start time of the log
    pub start_time: Option<DateTime<Utc>>,
    /// End time of the log
    pub end_time: Option<DateTime<Utc>>,
    /// Markers on the timeline
    pub markers: Vec<TimelineMarker>,
    /// Secondary density data (e.g., for errors)
    pub density_secondary: Vec<u32>,
    /// Tertiary density data (e.g., for warnings)
    pub density_tertiary: Vec<u32>,
}

impl Default for TimelineData {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineData {
    pub fn new() -> Self {
        Self {
            position: 0.0,
            zoom: 1.0,
            pan: 0.0,
            dragging: false,
            loop_start: None,
            loop_end: None,
            density: Vec::new(),
            start_time: None,
            end_time: None,
            markers: Vec::new(),
            density_secondary: Vec::new(),
            density_tertiary: Vec::new(),
        }
    }

    /// Set the time range from message timestamps
    pub fn set_time_range(&mut self, start: DateTime<Utc>, end: DateTime<Utc>) {
        self.start_time = Some(start);
        self.end_time = Some(end);
    }

    /// Get the current time based on position
    pub fn current_time(&self) -> Option<DateTime<Utc>> {
        if let (Some(start), Some(end)) = (self.start_time, self.end_time) {
            let duration = (end - start).num_milliseconds() as f64;
            let offset = duration * self.position as f64;
            Some(start + chrono::Duration::milliseconds(offset as i64))
        } else {
            None
        }
    }

    /// Get time at a specific position (0.0 to 1.0)
    pub fn time_at_position(&self, pos: f32) -> Option<DateTime<Utc>> {
        if let (Some(start), Some(end)) = (self.start_time, self.end_time) {
            let duration = (end - start).num_milliseconds() as f64;
            let offset = duration * pos as f64;
            Some(start + chrono::Duration::milliseconds(offset as i64))
        } else {
            None
        }
    }

    /// Seek to a specific time
    pub fn seek_to_time(&mut self, time: DateTime<Utc>) {
        if let (Some(start), Some(end)) = (self.start_time, self.end_time) {
            let total_duration = (end - start).num_milliseconds() as f64;
            if total_duration > 0.0 {
                let elapsed = (time - start).num_milliseconds() as f64;
                self.position = (elapsed / total_duration).clamp(0.0, 1.0) as f32;
            }
        }
    }

    /// Set position and return clamped value
    pub fn set_position(&mut self, pos: f32) -> f32 {
        self.position = pos.clamp(0.0, 1.0);
        self.position
    }

    /// Set zoom level
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(1.0, 100.0);
        // Adjust pan to keep position visible
        self.pan = self.pan.clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
    }

    /// Zoom in
    pub fn zoom_in(&mut self) {
        self.set_zoom(self.zoom * 1.5);
    }

    /// Zoom out
    pub fn zoom_out(&mut self) {
        self.set_zoom(self.zoom / 1.5);
    }

    /// Set loop region
    pub fn set_loop_region(&mut self, start: Option<f32>, end: Option<f32>) {
        self.loop_start = start.map(|v| v.clamp(0.0, 1.0));
        self.loop_end = end.map(|v| v.clamp(0.0, 1.0));
    }

    /// Clear loop region
    pub fn clear_loop_region(&mut self) {
        self.loop_start = None;
        self.loop_end = None;
    }

    /// Check if position is in loop region
    pub fn in_loop_region(&self) -> bool {
        if let (Some(start), Some(end)) = (self.loop_start, self.loop_end) {
            self.position >= start && self.position <= end
        } else {
            false
        }
    }

    /// Add a marker
    pub fn add_marker(&mut self, position: f32, label: &str, color: [f32; 4]) {
        self.markers.push(TimelineMarker::new(position, label, color));
    }

    /// Clear all markers
    pub fn clear_markers(&mut self) {
        self.markers.clear();
    }

    /// Build message density histogram from timestamps
    pub fn build_density(&mut self, timestamps: &[DateTime<Utc>], num_bins: usize) {
        if timestamps.is_empty() {
            self.density.clear();
            return;
        }

        // Find time range
        let min_time = timestamps.iter().min().copied();
        let max_time = timestamps.iter().max().copied();

        if let (Some(min), Some(max)) = (min_time, max_time) {
            self.set_time_range(min, max);

            let total_duration = (max - min).num_milliseconds() as f64;
            if total_duration <= 0.0 {
                return;
            }

            // Build histogram
            let mut density = vec![0u32; num_bins];
            for ts in timestamps {
                let elapsed = (*ts - min).num_milliseconds() as f64;
                let bin = ((elapsed / total_duration) * (num_bins - 1) as f64) as usize;
                let bin = bin.min(num_bins - 1);
                density[bin] += 1;
            }

            self.density = density;
        }
    }

    /// Convert position to visible position accounting for zoom/pan
    pub fn position_to_screen(&self, pos: f32, width: f32) -> f32 {
        let visible_range = 1.0 / self.zoom;
        let relative_pos = (pos - self.pan) / visible_range;
        relative_pos * width
    }

    /// Convert screen position to timeline position accounting for zoom/pan
    pub fn screen_to_position(&self, screen_x: f32, width: f32) -> f32 {
        let visible_range = 1.0 / self.zoom;
        (screen_x / width) * visible_range + self.pan
    }
}

/// Actions returned by timeline widgets
#[derive(Clone, Copy, Debug)]
pub enum TimelineAction {
    None,
    Seek(f32),
    Zoom(f32),
    LoopSet(f32, f32),
    LoopClear,
    // Playback controls
    Play,
    Pause,
    StepBack,
    StepForward,
}

/// Available timeline visual variants
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TimelineVariant {
    #[default]
    Minimal,
    Classic,
}

impl TimelineVariant {
    pub fn name(&self) -> &'static str {
        match self {
            TimelineVariant::Minimal => "Minimal",
            TimelineVariant::Classic => "Classic",
        }
    }

    pub fn all() -> &'static [TimelineVariant] {
        &[
            TimelineVariant::Minimal,
            TimelineVariant::Classic,
        ]
    }
}

/// Trait for timeline theme implementations
pub trait TimelineTheme {
    /// Get the name of this theme
    fn name(&self) -> &'static str;

    /// Render the timeline and return any action
    fn render(&mut self, ui: &Ui, data: &mut TimelineData) -> TimelineAction;
}

/// Timeline widget wrapper that delegates to the active variant
pub struct TimelineWidget {
    data: TimelineData,
    variant: TimelineVariant,
    classic: ClassicTimeline,
    minimal: MinimalTimeline,
}

impl Default for TimelineWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineWidget {
    pub fn new() -> Self {
        Self {
            data: TimelineData::new(),
            variant: TimelineVariant::default(),
            classic: ClassicTimeline::new(),
            minimal: MinimalTimeline::new(),
        }
    }

    /// Get mutable access to timeline data
    pub fn data_mut(&mut self) -> &mut TimelineData {
        &mut self.data
    }

    /// Get read access to timeline data
    pub fn data(&self) -> &TimelineData {
        &self.data
    }

    /// Get the current variant
    pub fn variant(&self) -> TimelineVariant {
        self.variant
    }

    /// Set the timeline variant
    pub fn set_variant(&mut self, variant: TimelineVariant) {
        self.variant = variant;
    }

    /// Convenience methods delegating to data

    pub fn set_time_range(&mut self, start: DateTime<Utc>, end: DateTime<Utc>) {
        self.data.set_time_range(start, end);
    }

    pub fn set_position(&mut self, pos: f32) {
        self.data.set_position(pos);
    }

    pub fn position(&self) -> f32 {
        self.data.position
    }

    pub fn current_time(&self) -> Option<DateTime<Utc>> {
        self.data.current_time()
    }

    pub fn seek_to_time(&mut self, time: DateTime<Utc>) {
        self.data.seek_to_time(time);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.data.set_zoom(zoom);
    }

    pub fn zoom_in(&mut self) {
        self.data.zoom_in();
    }

    pub fn zoom_out(&mut self) {
        self.data.zoom_out();
    }

    pub fn set_loop_region(&mut self, start: Option<f32>, end: Option<f32>) {
        self.data.set_loop_region(start, end);
    }

    pub fn clear_loop_region(&mut self) {
        self.data.clear_loop_region();
    }

    pub fn in_loop_region(&self) -> bool {
        self.data.in_loop_region()
    }

    pub fn add_marker(&mut self, position: f32, label: &str, color: [f32; 4]) {
        self.data.add_marker(position, label, color);
    }

    pub fn clear_markers(&mut self) {
        self.data.clear_markers();
    }

    pub fn build_density(&mut self, timestamps: &[DateTime<Utc>], num_bins: usize) {
        self.data.build_density(timestamps, num_bins);
    }

    /// Set the playing state (for playback button display)
    pub fn set_playing(&mut self, playing: bool) {
        self.minimal.set_playing(playing);
    }

    /// Render the timeline using the active variant
    pub fn render(&mut self, ui: &Ui) -> TimelineAction {
        match self.variant {
            TimelineVariant::Minimal => self.minimal.render(ui, &mut self.data),
            TimelineVariant::Classic => self.classic.render(ui, &mut self.data),
        }
    }
}

/// Timeline window wrapper
pub struct TimelineWindow {
    timeline: TimelineWidget,
    visible: bool,
}

impl Default for TimelineWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineWindow {
    pub fn new() -> Self {
        Self {
            timeline: TimelineWidget::new(),
            visible: true,
        }
    }

    pub fn timeline(&mut self) -> &mut TimelineWidget {
        &mut self.timeline
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn render(&mut self, ui: &Ui, is_open: &mut bool) -> TimelineAction {
        if !self.visible {
            return TimelineAction::None;
        }

        let mut action = TimelineAction::None;

        ui.window("Timeline")
            .size([1380.0, 150.0], imgui::Condition::FirstUseEver)
            .position([10.0, 860.0], imgui::Condition::FirstUseEver)
            .opened(is_open)
            .build(|| {
                action = self.timeline.render(ui);
            });

        action
    }

    /// Render content without window wrapper - for embedding in workspace
    pub fn render_content(&mut self, ui: &Ui, _width: f32, _height: f32) -> TimelineAction {
        if !self.visible {
            return TimelineAction::None;
        }
        self.timeline.render(ui)
    }
}
