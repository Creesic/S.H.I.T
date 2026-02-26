mod core;
mod decode;
mod hardware;
mod input;
mod playback;
mod ui;

use core::{CanMessage, DbcFile};
use decode::SignalDecoder;
use input::load_file;
use playback::PlaybackEngine;
use hardware::CanManager;
use hardware::can_interface::InterfaceType;
use ui::{MessageListWindow, FileDialogs, MultiSignalGraph, HardwareManagerWindow, LiveModeAction, LiveMessageWindow, MessageSenderWindow, MessageStatsWindow, PatternAnalyzerWindow, ShortcutManager, ExportDialog, AboutDialog, LiveModeState, BitVisualizerWindow, SignalInfo};
use chrono::{DateTime, Utc};
use imgui::{Context, FontConfig, FontSource, Condition};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;

use glutin::prelude::*;
use glutin::display::GetGlDisplay;
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasRawWindowHandle;
use glow::HasContext;

use std::time::Instant;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

struct AppState {
    messages: Vec<CanMessage>,
    playback: PlaybackEngine,
    message_list: MessageListWindow,
    charts: MultiSignalGraph,
    hardware_manager: HardwareManagerWindow,
    live_message_window: LiveMessageWindow,
    message_sender: MessageSenderWindow,
    initial_data_populated: bool,  // Track if we've done initial population
    // Phase 6 components
    message_stats: MessageStatsWindow,
    pattern_analyzer: PatternAnalyzerWindow,
    shortcut_manager: ShortcutManager,
    export_dialog: ExportDialog,
    about_dialog: AboutDialog,
    // Bit visualizer
    bit_visualizer: BitVisualizerWindow,
    dbc_file: DbcFile,
    signal_decoder: SignalDecoder,
    file_loaded: bool,
    dbc_loaded: bool,
    show_file_open_pending: bool,
    show_dbc_open_pending: bool,
    status_message: Option<String>,
    // Incremental chart data loading
    pending_signal_loads: std::collections::HashMap<String, usize>,  // signal_name -> current message index
    // Window visibility
    show_messages: bool,
    show_charts: bool,
    show_hardware_manager: bool,
    show_live_messages: bool,
    show_message_sender: bool,
    // Phase 6 window visibility
    show_message_stats: bool,
    show_pattern_analyzer: bool,
    show_shortcuts: bool,
    // Bit visualizer visibility
    show_bit_visualizer: bool,
    // CAN hardware manager
    can_manager: CanManager,
    // Async loading state
    loading: bool,
    loading_progress: f32,
    loading_total: usize,
    loading_receiver: Option<Receiver<LoadingUpdate>>,
    pending_messages: Option<Arc<Mutex<Vec<CanMessage>>>>,
}

/// Messages for async loading
enum LoadingUpdate {
    Progress(usize, usize),
    Complete(Vec<CanMessage>),
    Error(String),
}

/// Persistent application settings
#[derive(Serialize, Deserialize, Default)]
struct AppSettings {
    show_messages: bool,
    show_charts: bool,
    show_hardware_manager: bool,
    show_live_messages: bool,
    show_message_sender: bool,
    show_message_stats: bool,
    show_pattern_analyzer: bool,
    show_shortcuts: bool,
    show_bit_visualizer: bool,
}

impl AppSettings {
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("can-viz").join("settings.json"))
    }

    fn load() -> Self {
        if let Some(path) = Self::config_path() {
            if path.exists() {
                if let Ok(contents) = fs::read_to_string(&path) {
                    if let Ok(settings) = serde_json::from_str(&contents) {
                        return settings;
                    }
                }
            }
        }
        // Return default with bit visualizer enabled
        Self {
            show_messages: true,
            show_charts: true,
            show_bit_visualizer: true,
            ..Default::default()
        }
    }

    fn save(&self) {
        if let Some(path) = Self::config_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = fs::write(&path, json);
            }
        }
    }
}

impl AppState {
    fn new() -> Self {
        // Load persisted settings
        let settings = AppSettings::load();

        Self {
            messages: Vec::new(),
            playback: PlaybackEngine::new(Vec::new()),
            message_list: MessageListWindow::new(),
            charts: MultiSignalGraph::new(),
            hardware_manager: HardwareManagerWindow::new(),
            live_message_window: LiveMessageWindow::new(),
            message_sender: MessageSenderWindow::new(),
            initial_data_populated: false,
            // Phase 6 components
            message_stats: MessageStatsWindow::new(),
            pattern_analyzer: PatternAnalyzerWindow::new(),
            shortcut_manager: ShortcutManager::new(),
            export_dialog: ExportDialog::new(),
            about_dialog: AboutDialog::new(),
            // Bit visualizer
            bit_visualizer: BitVisualizerWindow::new(),
            dbc_file: DbcFile::new(),
            signal_decoder: SignalDecoder::new(),
            file_loaded: false,
            dbc_loaded: false,
            show_file_open_pending: false,
            show_dbc_open_pending: false,
            status_message: None,
            pending_signal_loads: std::collections::HashMap::new(),
            // Window visibility from settings
            show_messages: settings.show_messages,
            show_charts: settings.show_charts,
            show_hardware_manager: settings.show_hardware_manager,
            show_live_messages: settings.show_live_messages,
            show_message_sender: settings.show_message_sender,
            // Phase 6 window visibility
            show_message_stats: settings.show_message_stats,
            show_pattern_analyzer: settings.show_pattern_analyzer,
            show_shortcuts: settings.show_shortcuts,
            // Bit visualizer visibility
            show_bit_visualizer: settings.show_bit_visualizer,
            // CAN hardware manager
            can_manager: CanManager::new(),
            // Async loading
            loading: false,
            loading_progress: 0.0,
            loading_total: 0,
            loading_receiver: None,
            pending_messages: None,
        }
    }

    fn save_settings(&self) {
        let settings = AppSettings {
            show_messages: self.show_messages,
            show_charts: self.show_charts,
            show_hardware_manager: self.show_hardware_manager,
            show_live_messages: self.show_live_messages,
            show_message_sender: self.show_message_sender,
            show_message_stats: self.show_message_stats,
            show_pattern_analyzer: self.show_pattern_analyzer,
            show_shortcuts: self.show_shortcuts,
            show_bit_visualizer: self.show_bit_visualizer,
        };
        settings.save();
    }

    fn load_file(&mut self, path: &str) {
        // Start async loading
        self.loading = true;
        self.loading_progress = 0.0;
        self.loading_total = 0;
        self.status_message = Some(format!("Loading {}...", path));

        let path = path.to_string();
        let (tx, rx) = channel();
        self.loading_receiver = Some(rx);

        std::thread::spawn(move || {
            // Send progress updates during loading
            match load_file(&path) {
                Ok(messages) => {
                    let total = messages.len();
                    // Send progress updates
                    for (i, _) in messages.iter().enumerate() {
                        if i % 10000 == 0 {
                            let _ = tx.send(LoadingUpdate::Progress(i, total));
                        }
                    }
                    let _ = tx.send(LoadingUpdate::Complete(messages));
                }
                Err(e) => {
                    let _ = tx.send(LoadingUpdate::Error(e.to_string()));
                }
            }
        });
    }

    /// Process loading updates from background thread
    fn process_loading(&mut self) {
        // Take the receiver to avoid borrow issues
        let receiver = match self.loading_receiver.take() {
            Some(r) => r,
            None => return,
        };

        // Non-blocking check for updates
        let mut done = false;
        let mut should_restore = true;

        while let Ok(update) = receiver.try_recv() {
            match update {
                LoadingUpdate::Progress(current, total) => {
                    self.loading_progress = if total > 0 {
                        (current as f32 / total as f32) * 100.0
                    } else {
                        0.0
                    };
                    self.loading_total = total;
                    self.status_message = Some(format!(
                        "Loading... {:.0}% ({}/{})",
                        self.loading_progress, current, total
                    ));
                }
                LoadingUpdate::Complete(messages) => {
                    self.finish_loading(messages);
                    self.loading = false;
                    done = true;
                    should_restore = false;
                }
                LoadingUpdate::Error(e) => {
                    self.status_message = Some(format!("Failed to load file: {}", e));
                    self.loading = false;
                    done = true;
                    should_restore = false;
                }
            }
            if done {
                break;
            }
        }

        // Restore receiver if not done
        if should_restore {
            self.loading_receiver = Some(receiver);
        }
    }

    /// Finish loading after background thread completes
    fn finish_loading(&mut self, messages: Vec<CanMessage>) {
        let msg_count = messages.len();
        self.messages = messages.clone();
        self.playback = PlaybackEngine::new(messages.clone());
        self.message_list.set_messages(messages.clone());
        self.file_loaded = true;
        self.initial_data_populated = false;  // Reset for initial population

        // Set data time range for charts timeline
        if let (Some(first), Some(last)) = (messages.first(), messages.last()) {
            self.charts.set_data_time_range(first.timestamp, last.timestamp);
        }

        // Clear chart data but keep selected signals
        self.charts.clear_data();

        // Pre-populate chart with all data if DBC is already loaded
        if self.dbc_loaded {
            self.populate_chart_data();
        }

        // Update message statistics and pattern analyzer
        self.message_stats.update(&messages);
        self.pattern_analyzer.analyze(&messages);

        self.status_message = Some(format!("Loaded {} messages", msg_count));
        println!("Loaded {} messages", msg_count);
    }

    /// Pre-populate chart with all decoded signal data from loaded messages
    fn populate_chart_data(&mut self) {
        let charted: Vec<String> = self.charts.charted_signals().iter().map(|s| s.to_string()).collect();
        if charted.is_empty() {
            return;
        }

        for msg in &self.messages {
            let signals = self.signal_decoder.decode_message(&msg);
            for signal in &signals {
                if charted.contains(&signal.name) {
                    self.charts.add_point(&signal.name, signal.physical_value, msg.timestamp);
                }
            }
        }
    }

    /// Populate chart data for a specific signal
    fn populate_chart_data_for_signal(&mut self, signal_name: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/can-viz-chart-debug.txt")
            .ok();
        if let Some(ref mut f) = f {
            let _ = writeln!(f, "populate_chart_data_for_signal: {}", signal_name);
            let _ = writeln!(f, "  file_loaded: {}, dbc_loaded: {}", self.file_loaded, self.dbc_loaded);
        }

        if !self.file_loaded || !self.dbc_loaded {
            if let Some(ref mut f) = f { let _ = writeln!(f, "  returning early - files not loaded"); }
            return;
        }

        // Start incremental loading - begin at message index 0
        self.pending_signal_loads.insert(signal_name.to_string(), 0);
        if let Some(ref mut f) = f { let _ = writeln!(f, "  started incremental loading for {}", signal_name); }
    }

    // Process a batch of pending signal data loading (call this each frame)
    fn process_pending_signal_loads(&mut self) {
        const BATCH_SIZE: usize = 10000; // Process up to 10k messages per frame per signal

        let mut completed = Vec::new();

        for (signal_name, start_idx) in self.pending_signal_loads.iter_mut() {
            let end_idx = (*start_idx + BATCH_SIZE).min(self.messages.len());

            for msg_idx in *start_idx..end_idx {
                if let Some(msg) = self.messages.get(msg_idx) {
                    let signals = self.signal_decoder.decode_message(&msg);
                    for signal in &signals {
                        if signal.name == *signal_name {
                            self.charts.add_point(&signal.name, signal.physical_value, msg.timestamp);
                        }
                    }
                }
            }

            *start_idx = end_idx;

            if end_idx >= self.messages.len() {
                completed.push(signal_name.clone());
            }
        }

        // Remove completed loads
        for name in completed {
            self.pending_signal_loads.remove(&name);
        }
    }

    fn load_dbc(&mut self, path: &str) {
        match DbcFile::load(path) {
            Ok(dbc) => {
                self.signal_decoder.set_dbc(dbc.clone());
                self.dbc_file = dbc.clone();
                self.message_list.set_dbc(dbc.clone());
                self.dbc_loaded = true;

                // Populate available signals for charts
                let mut signals = Vec::new();
                for msg in &dbc.messages {
                    for sig in &msg.signals {
                        signals.push(SignalInfo {
                            name: sig.name.clone(),
                            msg_id: msg.id,
                            msg_name: msg.name.clone(),
                            unit: sig.unit.clone().unwrap_or_default(),
                        });
                    }
                }
                self.charts.set_available_signals(signals);

                // Pre-populate chart with all data if log file is already loaded
                if self.file_loaded {
                    self.populate_chart_data();
                }

                self.status_message = Some(format!("Loaded DBC: {} messages defined", self.dbc_file.messages.len()));
                println!("Loaded DBC with {} messages", self.dbc_file.messages.len());
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to load DBC: {}", e));
                eprintln!("Failed to load DBC: {}", e);
            }
        }
    }

    fn process_file_dialogs(&mut self) {
        // Handle file open dialog
        if self.show_file_open_pending {
            if let Some(path) = FileDialogs::open_can_file() {
                self.load_file(path.to_str().unwrap_or(""));
            }
            self.show_file_open_pending = false;
        }

        // Handle DBC open dialog
        if self.show_dbc_open_pending {
            if let Some(path) = FileDialogs::open_dbc_file() {
                self.load_dbc(path.to_str().unwrap_or(""));
            }
            self.show_dbc_open_pending = false;
        }
    }

    fn update_graphs(&mut self) {
        if !self.file_loaded {
            return;
        }

        // Update when playing, or do initial population once when stopped/paused
        let is_initial_pop = !self.initial_data_populated && self.playback.current_time().is_some();
        if !self.playback.is_playing() && !is_initial_pop {
            return;
        }

        if let Some(_current_time) = self.playback.current_time() {
            let window_msgs = self.playback.get_window(
                chrono::Duration::milliseconds(100),
                chrono::Duration::seconds(0),
            );

            // Update message list (live mode)
            for msg in window_msgs {
                self.message_list.update_message(msg);
            }
        }

        // Mark initial population as done
        if !self.playback.is_playing() {
            self.initial_data_populated = true;
        }
    }
}

fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create tokio runtime for async operations
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    // Create event loop
    let event_loop = EventLoop::new().expect("Failed to create EventLoop");

    // Build the window and GL display using glutin-winit
    let (window, gl_config) = DisplayBuilder::new()
        .with_window_builder(Some(
            WindowBuilder::new()
                .with_title("CAN-Viz - CAN Bus Visualization Tool")
                .with_inner_size(winit::dpi::LogicalSize::new(1400.0, 900.0))
        ))
        .build(&event_loop, glutin::config::ConfigTemplateBuilder::new(), |mut iter| {
            iter.next().unwrap()
        })
        .expect("Failed to create window and display");

    let window = window.expect("Failed to create window");
    let gl_display = gl_config.display();

    // Create the context using the proper API
    let context = unsafe {
        gl_display.create_context(
            &gl_config,
            &glutin::context::ContextAttributesBuilder::new()
                .build(Some(window.raw_window_handle())),
        )
    }.expect("Failed to create GL context");

    // Create surface and make context current
    let attrs = window.build_surface_attributes(
        glutin::surface::SurfaceAttributesBuilder::<glutin::surface::WindowSurface>::new()
    );

    let surface = unsafe {
        gl_display.create_window_surface(&gl_config, &attrs)
    }.expect("Failed to create surface");

    let context = context.make_current(&surface).expect("Failed to make context current");

    // Create glow context for renderer
    let gl = unsafe {
        glow::Context::from_loader_function(|ptr| {
            gl_display.get_proc_address(&std::ffi::CString::new(ptr).unwrap()) as *const _
        })
    };

    // Set up imgui
    let mut imgui = Context::create();

    // Disable ImGui debug log window
    imgui.set_log_filename(None::<std::path::PathBuf>);

    // Re-enable ini file for saving window positions
    let ini_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("can-viz")
        .join("layout.ini");

    if let Some(parent) = ini_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    imgui.set_ini_filename(Some(ini_path));

    // Disable debug log via FFI
    unsafe {
        // Get current context and set ShowDebugLog to false
        let ctx = imgui::sys::igGetCurrentContext();
        if !ctx.is_null() {
            // The DebugMetricsConfig.ShowDebugLog field controls the debug log window
            (*ctx).DebugMetricsConfig.ShowDebugLog = false;
        }
    }

    // Enable docking
    imgui.io_mut().config_flags |= imgui::ConfigFlags::DOCKING_ENABLE;

    // Configure fonts
    let hidpi_factor = window.scale_factor();
    let font_size = (14.0 * hidpi_factor) as f32;
    imgui.fonts().add_font(&[FontSource::DefaultFontData {
        config: Some(FontConfig {
            size_pixels: font_size,
            ..FontConfig::default()
        }),
    }]);
    imgui.io_mut().font_global_scale = (1.0 / hidpi_factor) as f32;

    // Set up platform and renderer
    let mut platform = WinitPlatform::init(&mut imgui);
    platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Default);

    let mut renderer = imgui_glow_renderer::AutoRenderer::initialize(gl, &mut imgui)
        .expect("Failed to initialize renderer");

    // Create a second glow context for clearing (both reference the same GL context)
    let gl_clear = unsafe {
        glow::Context::from_loader_function(|ptr| {
            gl_display.get_proc_address(&std::ffi::CString::new(ptr).unwrap()) as *const _
        })
    };

    // Create app state
    let mut state = AppState::new();
    let mut last_frame_time = Instant::now();
    let mut last_settings_save = Instant::now();

    // Main loop
    event_loop.run(move |event, window_target| {
        match event {
            Event::NewEvents(_) => {
                let now = Instant::now();
                imgui.io_mut().update_delta_time(now - last_frame_time);
                last_frame_time = now;
            }
            Event::AboutToWait => {
                // Process file dialogs
                state.process_file_dialogs();

                // Process async loading
                state.process_loading();

                // Update playback
                state.playback.update(std::time::Duration::from_millis(16));

                // Update graphs with decoded signals
                state.update_graphs();

                // Save settings periodically (every 30 seconds)
                if last_settings_save.elapsed().as_secs() >= 30 {
                    state.save_settings();
                    last_settings_save = Instant::now();
                }

                platform.prepare_frame(imgui.io_mut(), &window)
                    .expect("Failed to prepare frame");
                window.request_redraw();
            }
            Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                let ui = imgui.new_frame();

                // Hide the Debug window by moving it off-screen and collapsing it
                unsafe {
                    use std::ffi::CString;
                    let debug_window_name = CString::new("Debug##Default").unwrap();

                    // Move the Debug window way off-screen
                    let off_screen_pos = imgui::sys::ImVec2 { x: -10000.0, y: -10000.0 };
                    imgui::sys::igSetWindowPos_Str(
                        debug_window_name.as_ptr(),
                        off_screen_pos,
                        imgui::sys::ImGuiCond_Always as imgui::sys::ImGuiCond
                    );

                    // Collapse it
                    imgui::sys::igSetWindowCollapsed_Str(
                        debug_window_name.as_ptr(),
                        true,
                        imgui::sys::ImGuiCond_Always as imgui::sys::ImGuiCond
                    );

                    // Also clear the debug log buffer
                    let ctx = imgui::sys::igGetCurrentContext();
                    if !ctx.is_null() {
                        (*ctx).DebugMetricsConfig.ShowDebugLog = false;
                        imgui::sys::ImGuiTextBuffer_clear(&mut (*ctx).DebugLogBuf);
                    }
                }

                // Menu bar
                ui.main_menu_bar(|| {
                    ui.menu("File", || {
                        if ui.menu_item("Open CAN Log...") {
                            state.show_file_open_pending = true;
                        }
                        if ui.menu_item("Load DBC...") {
                            state.show_dbc_open_pending = true;
                        }
                        ui.separator();
                        if ui.menu_item("Exit") {
                            window_target.exit();
                        }
                    });

                    ui.menu("Playback", || {
                        if ui.menu_item("Play") {
                            state.playback.play();
                        }
                        if ui.menu_item("Pause") {
                            state.playback.pause();
                        }
                        if ui.menu_item("Stop") {
                            state.playback.stop();
                        }
                        ui.separator();
                        ui.text(format!("Speed: {:.1}x", state.playback.speed()));
                    });

                    ui.menu("View", || {
                        let _tok = if state.show_messages { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Messages") {
                            state.show_messages = !state.show_messages;
                        }
                        drop(_tok);

                        let _tok = if state.show_charts { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Charts") {
                            state.show_charts = !state.show_charts;
                        }
                        drop(_tok);

                        ui.separator();

                        // Hardware/Live Mode windows
                        let _tok = if state.show_hardware_manager { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Hardware Manager") {
                            state.show_hardware_manager = !state.show_hardware_manager;
                        }
                        drop(_tok);

                        let _tok = if state.show_live_messages { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Live Messages") {
                            state.show_live_messages = !state.show_live_messages;
                        }
                        drop(_tok);

                        let _tok = if state.show_message_sender { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Message Sender") {
                            state.show_message_sender = !state.show_message_sender;
                        }
                        drop(_tok);

                        ui.separator();

                        // Analysis windows
                        let _tok = if state.show_message_stats { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Message Statistics") {
                            state.show_message_stats = !state.show_message_stats;
                        }
                        drop(_tok);

                        let _tok = if state.show_pattern_analyzer { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Pattern Analyzer") {
                            state.show_pattern_analyzer = !state.show_pattern_analyzer;
                        }
                        drop(_tok);

                        ui.separator();

                        // Bit Visualizer
                        let _tok = if state.show_bit_visualizer { Some(ui.push_style_color(imgui::StyleColor::Text, [0.0, 1.0, 0.0, 1.0])) } else { None };
                        if ui.menu_item("Bit Visualizer") {
                            state.show_bit_visualizer = !state.show_bit_visualizer;
                        }
                        drop(_tok);
                    });

                    ui.menu("Help", || {
                        if ui.menu_item("Keyboard Shortcuts") {
                            state.show_shortcuts = true;
                        }
                        ui.separator();
                        if ui.menu_item("About CAN-Viz") {
                            state.about_dialog.show();
                        }
                    });
                });

                // Status bar
                let window_size = window.inner_size();
                ui.set_cursor_pos([0.0, window_size.height as f32 / hidpi_factor as f32 - 25.0]);
                ui.child_window("Status")
                    .size([window_size.width as f32 / hidpi_factor as f32, 25.0])
                    .build(|| {
                        if state.loading {
                            // Show loading progress
                            ui.text_colored([1.0, 0.8, 0.3, 1.0],
                                format!("Loading... {:.0}% ({})", state.loading_progress, state.loading_total)
                            );
                        } else if let Some(ref msg) = state.status_message {
                            ui.text(msg);
                        } else if state.file_loaded {
                            ui.text(format!(
                                "Messages: {} | DBC: {} | Position: {}",
                                state.messages.len(),
                                if state.dbc_loaded { "Loaded" } else { "None" },
                                state.playback.position()
                            ));
                        } else {
                            ui.text("Open a CAN log file to begin (File > Open CAN Log...)");
                        }
                    });

                // Loading overlay
                if state.loading {
                    let screen_center = [
                        window_size.width as f32 / hidpi_factor as f32 / 2.0,
                        window_size.height as f32 / hidpi_factor as f32 / 2.0,
                    ];

                    // Semi-transparent background
                    let draw_list = ui.get_background_draw_list();
                    draw_list.add_rect(
                        [0.0, 0.0],
                        [window_size.width as f32 / hidpi_factor as f32, window_size.height as f32 / hidpi_factor as f32],
                        [0.0, 0.0, 0.0, 0.5],
                    ).filled(true).build();

                    // Loading text
                    let loading_text = format!("Loading... {:.0}%", state.loading_progress);
                    let text_width = loading_text.len() as f32 * 8.0;
                    draw_list.add_text(
                        [screen_center[0] - text_width / 2.0, screen_center[1]],
                        [1.0, 1.0, 1.0, 1.0],
                        loading_text,
                    );

                    // Progress bar
                    let bar_width = 200.0;
                    let bar_height = 10.0;
                    let bar_x = screen_center[0] - bar_width / 2.0;
                    let bar_y = screen_center[1] + 25.0;

                    draw_list.add_rect(
                        [bar_x, bar_y],
                        [bar_x + bar_width, bar_y + bar_height],
                        [0.3, 0.3, 0.3, 1.0],
                    ).filled(true).rounding(3.0).build();

                    let progress_width = bar_width * (state.loading_progress / 100.0);
                    draw_list.add_rect(
                        [bar_x, bar_y],
                        [bar_x + progress_width, bar_y + bar_height],
                        [0.3, 0.7, 1.0, 1.0],
                    ).filled(true).rounding(3.0).build();
                }

                // Create a dockspace over the main viewport
                // This allows windows to be docked/rearranged within the main window
                // Windows can be dragged and docked to different areas, but stay within the app
                ui.dockspace_over_main_viewport();

                // Render windows - these will dock into the dockspace above
                // Windows can be rearranged by dragging their tabs/bars

                if state.show_messages {
                    state.message_list.render(&ui, &mut state.show_messages);
                }

                if state.show_charts {
                    // Process incremental data loading
                    state.process_pending_signal_loads();

                    let current_time = state.playback.current_time();
                    ui.window("Charts")
                        .size([600.0, 350.0], Condition::FirstUseEver)
                        .position([400.0, 30.0], Condition::FirstUseEver)
                        .opened(&mut state.show_charts)
                        .build(|| {
                            state.charts.render(ui, current_time, state.playback.is_playing());
                        });

                    // Handle seek request from chart click
                    // All values from chart are relative offsets from current time
                    // Positive = forward, Negative = backward
                    if let Some(offset_secs) = state.charts.take_seek_request() {
                        if let Some(current) = state.playback.current_time() {
                            let new_time = current + chrono::Duration::milliseconds((offset_secs * 1000.0) as i64);
                            state.playback.seek_to_time(Some(new_time));
                        }
                    }
                }

                // Hardware Manager with action handling
                if state.show_hardware_manager {
                    let action = state.hardware_manager.render(&ui, &mut state.show_hardware_manager);
                    match action {
                        LiveModeAction::Connect { interface, config } => {
                            eprintln!("[CAN-Viz] Connect button clicked!");
                            eprintln!("[CAN-Viz] Interface: {}", interface);
                            eprintln!("[CAN-Viz] Bitrate: {}", config.bitrate);
                            eprintln!("[CAN-Viz] Listen only: {}", config.listen_only);

                            // Determine interface type
                            let interface_type = if interface.starts_with("mock://") {
                                eprintln!("[CAN-Viz] Interface type: Virtual (mock)");
                                InterfaceType::Virtual
                            } else {
                                eprintln!("[CAN-Viz] Interface type: Serial");
                                InterfaceType::Serial
                            };

                            // Connect to the CAN interface
                            eprintln!("[CAN-Viz] Calling can_manager.connect()...");
                            let result = rt.block_on(state.can_manager.connect(
                                &interface,
                                crate::hardware::can_interface::CanConfig {
                                    bitrate: config.bitrate,
                                    fd_mode: false,
                                    listen_only: config.listen_only,
                                },
                                interface_type,
                            ));

                            eprintln!("[CAN-Viz] Connect result: {:?}", result);
                            match result {
                                Ok(()) => {
                                    eprintln!("[CAN-Viz] Connected successfully!");
                                    state.status_message = Some(format!("Connected to {}", interface));
                                }
                                Err(e) => {
                                    eprintln!("[CAN-Viz] Connection FAILED: {}", e);
                                    state.status_message = Some(format!("Failed to connect: {}", e));
                                }
                            }
                        }
                        LiveModeAction::Disconnect => {
                            println!("Disconnect from interface");
                            rt.block_on(state.can_manager.disconnect());
                            state.status_message = Some("Disconnected from CAN interface".to_string());
                        }
                        LiveModeAction::SendMessage { id, data } => {
                            println!("Send message: 0x{:03X} {:?}", id, data);
                            let msg = CanMessage::new(0, id, data);
                            let _ = rt.block_on(state.can_manager.send(msg));
                        }
                        LiveModeAction::StartRecording => {
                            eprintln!("[CAN-Viz] Recording started");
                            state.status_message = Some("Recording started".to_string());
                        }
                        LiveModeAction::StopRecording => {
                            let live_state = state.hardware_manager.state();
                            let msg_count = live_state.live_messages.len();
                            eprintln!("[CAN-Viz] Recording stopped - {} messages captured", msg_count);

                            if !live_state.live_messages.is_empty() {
                                // Convert live messages to CanMessage format and load into main state
                                let recorded_messages: Vec<CanMessage> = live_state.live_messages
                                    .iter()
                                    .map(|lm| CanMessage {
                                        timestamp: lm.timestamp,
                                        bus: lm.bus,
                                        id: lm.id,
                                        data: lm.data.clone(),
                                    })
                                    .collect();

                                // Load into main state
                                state.messages = recorded_messages.clone();
                                state.playback = PlaybackEngine::new(recorded_messages.clone());
                                state.message_list.set_messages(recorded_messages);
                                state.file_loaded = true;
                                state.initial_data_populated = false;

                                // Update charts time range based on recording
                                if let (Some(first), Some(last)) = (state.messages.first(), state.messages.last()) {
                                    state.charts.set_data_time_range(first.timestamp, last.timestamp);
                                }

                                // Pre-populate charts if DBC is loaded
                                if state.dbc_loaded {
                                    state.populate_chart_data();
                                }

                                eprintln!("[CAN-Viz] Loaded {} recorded messages into playback", state.messages.len());
                            }

                            state.status_message = Some(format!("Recording stopped - {} messages loaded into playback", msg_count));
                        }
                        LiveModeAction::SaveData => {
                            eprintln!("[CAN-Viz] Save data requested - {} messages", state.hardware_manager.state().live_messages.len());
                            // Save to CSV file
                            let live_state = state.hardware_manager.state();
                            if let Some(path) = crate::ui::FileDialogs::export_csv_file() {
                                match std::fs::File::create(&path) {
                                    Ok(mut file) => {
                                        use std::io::Write;
                                        // Write CSV header matching 130b.csv format
                                        let _ = writeln!(file, "time,addr,bus,data");
                                        // Get start time for relative timestamps
                                        let start_time = live_state.live_messages.first()
                                            .map(|m| m.timestamp);
                                        // Write messages
                                        for msg in &live_state.live_messages {
                                            // Calculate relative time in seconds
                                            let rel_time = if let Some(start) = start_time {
                                                (msg.timestamp - start).num_milliseconds() as f64 / 1000.0
                                            } else {
                                                0.0
                                            };
                                            // Data as hex string with 0x prefix
                                            let data_hex = if msg.data.is_empty() {
                                                "0x".to_string()
                                            } else {
                                                format!("0x{}", msg.data.iter()
                                                    .map(|b| format!("{:02X}", b))
                                                    .collect::<String>())
                                            };
                                            let _ = writeln!(file, "{:.3},0x{:03X},{},{}",
                                                rel_time, msg.id, msg.bus, data_hex);
                                        }
                                        state.status_message = Some(format!("Saved {} messages to {}", live_state.live_messages.len(), path.display()));
                                        eprintln!("[CAN-Viz] Saved {} messages to {}", live_state.live_messages.len(), path.display());
                                    }
                                    Err(e) => {
                                        state.status_message = Some(format!("Failed to save: {}", e));
                                        eprintln!("[CAN-Viz] Failed to save: {}", e);
                                    }
                                }
                            }
                        }
                        LiveModeAction::None => {}
                    }
                }

                // Update live messages from CAN manager
                if state.show_live_messages || state.hardware_manager.state().is_active {
                    // Poll for new messages (even if window is closed, we need to process them for charts)
                    let messages = rt.block_on(state.can_manager.get_messages());
                    let live_state = state.hardware_manager.state_mut();
                    let is_recording = live_state.is_recording;

                    // Update live state with received messages - only add to buffer if recording
                    for msg in &messages {
                        // Only store messages if recording is active
                        if is_recording {
                            live_state.add_message(msg.message.id, msg.message.data.clone(), msg.message.bus);
                        }

                        // Always update statistics
                        live_state.stats.messages_received += 1;

                        // Decode and add to charts if signals are charted
                        let decoded = state.signal_decoder.decode_message(&msg.message);
                        for signal in &decoded {
                            if state.charts.has_signal(&signal.name) {
                                state.charts.add_point(&signal.name, signal.physical_value, msg.timestamp);
                            }
                        }
                    }

                    if state.show_live_messages {
                        let live_state_ref = state.hardware_manager.state();
                        state.live_message_window.render(&ui, live_state_ref, &mut state.show_live_messages);
                    }
                }

                // Message Sender window
                if state.show_message_sender {
                    let is_connected = state.hardware_manager.state().is_active;
                    if let Some((id, data)) = state.message_sender.render(&ui, is_connected, &mut state.show_message_sender) {
                        println!("Send CAN message: 0x{:03X} {:?}", id, data);
                        // TODO: Actually send the message through the interface
                    }
                }

                // Message Statistics window
                if state.show_message_stats {
                    state.message_stats.render(&ui, &mut state.show_message_stats);
                }

                // Pattern Analyzer window
                if state.show_pattern_analyzer {
                    state.pattern_analyzer.render(&ui, &mut state.show_pattern_analyzer);
                }

                // Bit Visualizer window - update with selected message
                if state.show_bit_visualizer {
                    // Update visualizer with currently selected message from message list
                    use std::io::Write;
                    let selected = state.message_list.selected_message();
                    let debug_info = state.message_list.debug_info();
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/tmp/can-viz-chart-debug.txt")
                        .ok();
                    if let Some(ref mut f) = f {
                        let _ = writeln!(f, "=== message_list check ===");
                        let _ = writeln!(f, "  selected_id: {:?}", debug_info.0);
                        let _ = writeln!(f, "  states count: {}", debug_info.1);
                        let _ = writeln!(f, "  messages count: {}", debug_info.2);
                    }

                    if let Some(selected_msg) = selected {
                        state.bit_visualizer.set_message(selected_msg.id, &selected_msg.data);
                    }

                    // Get list of charted signals
                    let charted: Vec<String> = state.charts.get_charted_signals();
                    state.bit_visualizer.set_charted_signals(charted);

                    state.bit_visualizer.render(&ui, &mut state.dbc_file, &mut state.show_bit_visualizer);

                    // Check for chart toggle requests
                    if let Some(signal_name) = state.bit_visualizer.take_chart_toggle_request() {
                        use std::io::Write;
                        let mut f = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("/tmp/can-viz-chart-debug.txt")
                            .ok();
                        if let Some(ref mut f) = f {
                            let _ = writeln!(f, "main.rs: received chart toggle request for: {}", signal_name);
                        }

                        let was_charted = state.charts.has_signal(&signal_name);
                        if let Some(ref mut f) = f { let _ = writeln!(f, "  was_charted: {}", was_charted); }

                        state.charts.toggle_signal_by_name(&signal_name);
                        // If signal was newly added, populate its data
                        if !was_charted {
                            state.populate_chart_data_for_signal(&signal_name);
                        }
                    }

                    // Sync DBC changes to other components
                    state.signal_decoder.set_dbc(state.dbc_file.clone());
                }

                // Keyboard Shortcuts help window
                if state.show_shortcuts {
                    state.shortcut_manager.render_help(&ui, &mut state.show_shortcuts);
                }

                // Export Dialog
                if let Some(_export_request) = state.export_dialog.render(&ui) {
                    // TODO: Implement actual export functionality
                    println!("Export requested");
                }

                // About Dialog
                state.about_dialog.render(&ui);

                // Prepare and render
                platform.prepare_render(&ui, &window);
                let draw_data = imgui.render();

                // Clear the screen before rendering
                unsafe {
                    gl_clear.clear_color(0.1, 0.1, 0.1, 1.0); // Dark gray background
                    gl_clear.clear(glow::COLOR_BUFFER_BIT);
                }

                renderer.render(draw_data).expect("Rendering failed");

                surface.swap_buffers(&context).expect("Failed to swap buffers");
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                state.save_settings();
                window_target.exit();
            }
            _ => {}
        }

        platform.handle_event(imgui.io_mut(), &window, &event);
    }).expect("EventLoop error");
}
