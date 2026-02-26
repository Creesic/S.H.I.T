use crate::core::CanMessage;
use crate::hardware::can_interface::{CanConfig, CanInterface, InterfaceType};
use crate::hardware::serial_can::SerialCanInterface;
use crate::hardware::mock::MockCanInterface;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::VecDeque;
use tokio::sync::{mpsc, Mutex};
use chrono::Utc;

/// Maximum messages to keep in the live buffer
const MAX_LIVE_MESSAGES: usize = 5000;

/// Message from the CAN manager to the UI
#[derive(Clone)]
pub struct ManagerMessage {
    pub message: CanMessage,
    pub timestamp: chrono::DateTime<Utc>,
}

/// CAN hardware manager that handles connections and message streaming
pub struct CanManager {
    /// Current connection status
    status: Arc<Mutex<ConnectionStatus>>,
    /// Received messages buffer
    messages: Arc<Mutex<VecDeque<ManagerMessage>>>,
    /// Statistics
    stats: Arc<ManagerStats>,
    /// Stop signal for background task
    stop_signal: Arc<AtomicBool>,
    /// TX channel for sending messages
    tx_sender: Option<mpsc::Sender<CanMessage>>,
    /// Current interface name
    interface_name: Arc<Mutex<Option<String>>>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Default)]
pub struct ManagerStats {
    pub messages_received: AtomicU64,
    pub messages_sent: AtomicU64,
    pub errors: AtomicU64,
    pub start_time: Arc<Mutex<Option<chrono::DateTime<Utc>>>>,
}

impl Default for CanManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CanManager {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(ConnectionStatus::Disconnected)),
            messages: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LIVE_MESSAGES))),
            stats: Arc::new(ManagerStats::default()),
            stop_signal: Arc::new(AtomicBool::new(false)),
            tx_sender: None,
            interface_name: Arc::new(Mutex::new(None)),
        }
    }

    /// Get current connection status
    pub async fn status(&self) -> ConnectionStatus {
        *self.status.lock().await
    }

    /// Get the interface name
    pub async fn interface_name(&self) -> Option<String> {
        self.interface_name.lock().await.clone()
    }

    /// Connect to a CAN interface
    pub async fn connect(&mut self, interface: &str, config: CanConfig, interface_type: InterfaceType) -> Result<(), String> {
        self.connect_with_bus(interface, config, interface_type, 0).await
    }

    /// Connect to a CAN interface with a specific bus ID
    pub async fn connect_with_bus(
        &mut self,
        interface: &str,
        config: CanConfig,
        interface_type: InterfaceType,
        bus_id: u8,
    ) -> Result<(), String> {
        // Set connecting status
        *self.status.lock().await = ConnectionStatus::Connecting;

        // Store interface name
        *self.interface_name.lock().await = Some(interface.to_string());

        // Clear previous messages
        self.messages.lock().await.clear();

        // Reset stats
        self.stats.messages_received.store(0, Ordering::SeqCst);
        self.stats.messages_sent.store(0, Ordering::SeqCst);
        self.stats.errors.store(0, Ordering::SeqCst);
        *self.stats.start_time.lock().await = Some(Utc::now());

        // Reset stop signal
        self.stop_signal.store(false, Ordering::SeqCst);

        // Create channels for message passing
        let (tx_sender, tx_receiver) = mpsc::channel::<CanMessage>(100);
        let (rx_sender, rx_receiver) = mpsc::channel::<CanMessage>(1000);

        self.tx_sender = Some(tx_sender);

        // Clone for async task
        let status = self.status.clone();
        let messages = self.messages.clone();
        let stats = self.stats.clone();
        let stop_signal = self.stop_signal.clone();
        let interface_str = interface.to_string();

        // Spawn background task for CAN communication
        tokio::spawn(async move {
            let result = match interface_type {
                InterfaceType::Serial => {
                    Self::run_serial_connection(
                        &interface_str,
                        config,
                        tx_receiver,
                        rx_sender,
                        status.clone(),
                        messages.clone(),
                        stats.clone(),
                        stop_signal.clone(),
                        bus_id,
                    ).await
                }
                InterfaceType::Virtual => {
                    Self::run_mock_connection(
                        &interface_str,
                        config,
                        tx_receiver,
                        rx_sender,
                        status.clone(),
                        messages.clone(),
                        stats.clone(),
                        stop_signal.clone(),
                        bus_id,
                    ).await
                }
                _ => Err("Unsupported interface type".to_string()),
            };

            if let Err(e) = result {
                *status.lock().await = ConnectionStatus::Error;
                eprintln!("CAN connection error: {}", e);
            }
        });

        // Spawn task to receive messages and add to buffer
        let messages_clone = self.messages.clone();
        let stats_clone = self.stats.clone();
        tokio::spawn(async move {
            let mut rx_receiver = rx_receiver;
            while let Some(msg) = rx_receiver.recv().await {
                let manager_msg = ManagerMessage {
                    message: msg,
                    timestamp: Utc::now(),
                };

                let mut msgs = messages_clone.lock().await;
                if msgs.len() >= MAX_LIVE_MESSAGES {
                    msgs.pop_front();
                }
                msgs.push_back(manager_msg);
                stats_clone.messages_received.fetch_add(1, Ordering::SeqCst);
            }
        });

        Ok(())
    }

    async fn run_serial_connection(
        interface: &str,
        config: CanConfig,
        mut tx_receiver: mpsc::Receiver<CanMessage>,
        rx_sender: mpsc::Sender<CanMessage>,
        status: Arc<Mutex<ConnectionStatus>>,
        _messages: Arc<Mutex<VecDeque<ManagerMessage>>>,
        stats: Arc<ManagerStats>,
        stop_signal: Arc<AtomicBool>,
        bus_id: u8,
    ) -> Result<(), String> {
        let mut can_if = SerialCanInterface::new_with_bus(interface, bus_id);

        // Connect to the interface
        can_if.connect(config.clone())
            .await
            .map_err(|e| format!("Failed to connect: {}", e))?;

        *status.lock().await = ConnectionStatus::Connected;

        // Main loop
        loop {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }

            // Try to receive messages
            match can_if.receive().await {
                Ok(Some(msg)) => {
                    if rx_sender.send(msg).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    // No message available, small delay
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                }
                Err(e) => {
                    stats.errors.fetch_add(1, Ordering::SeqCst);
                    eprintln!("Receive error: {}", e);
                }
            }

            // Try to send pending messages
            match tx_receiver.try_recv() {
                Ok(msg) => {
                    if let Err(e) = can_if.send(&msg).await {
                        stats.errors.fetch_add(1, Ordering::SeqCst);
                        eprintln!("Send error: {}", e);
                    } else {
                        stats.messages_sent.fetch_add(1, Ordering::SeqCst);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        // Disconnect
        let _ = can_if.disconnect().await;
        *status.lock().await = ConnectionStatus::Disconnected;

        Ok(())
    }

    async fn run_mock_connection(
        interface: &str,
        config: CanConfig,
        mut tx_receiver: mpsc::Receiver<CanMessage>,
        rx_sender: mpsc::Sender<CanMessage>,
        status: Arc<Mutex<ConnectionStatus>>,
        _messages: Arc<Mutex<VecDeque<ManagerMessage>>>,
        stats: Arc<ManagerStats>,
        stop_signal: Arc<AtomicBool>,
        bus_id: u8,
    ) -> Result<(), String> {
        let mut can_if = MockCanInterface::new_with_bus(interface, bus_id);
        can_if.set_auto_generate(true);

        can_if.connect(config)
            .await
            .map_err(|e| format!("Failed to connect: {}", e))?;

        *status.lock().await = ConnectionStatus::Connected;

        loop {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }

            // Receive from mock (generates random messages)
            match can_if.receive().await {
                Ok(Some(msg)) => {
                    if rx_sender.send(msg).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
                Err(e) => {
                    stats.errors.fetch_add(1, Ordering::SeqCst);
                    eprintln!("Mock receive error: {}", e);
                }
            }

            // Send pending messages
            match tx_receiver.try_recv() {
                Ok(msg) => {
                    if let Err(e) = can_if.send(&msg).await {
                        stats.errors.fetch_add(1, Ordering::SeqCst);
                        eprintln!("Mock send error: {}", e);
                    } else {
                        stats.messages_sent.fetch_add(1, Ordering::SeqCst);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        let _ = can_if.disconnect().await;
        *status.lock().await = ConnectionStatus::Disconnected;

        Ok(())
    }

    /// Disconnect from the CAN interface
    pub async fn disconnect(&mut self) {
        self.stop_signal.store(true, Ordering::SeqCst);
        self.tx_sender = None;
        *self.status.lock().await = ConnectionStatus::Disconnected;
        *self.interface_name.lock().await = None;
    }

    /// Send a CAN message
    pub async fn send(&self, message: CanMessage) -> Result<(), String> {
        if let Some(sender) = &self.tx_sender {
            sender.send(message).await
                .map_err(|e| format!("Failed to send: {}", e))?;
        }
        Ok(())
    }

    /// Get all received messages and clear the buffer
    pub async fn get_messages(&self) -> Vec<ManagerMessage> {
        std::mem::take(&mut *self.messages.lock().await).into_iter().collect()
    }

    /// Clear received messages
    pub async fn clear_messages(&self) {
        self.messages.lock().await.clear();
    }

    /// Get statistics
    pub fn get_stats(&self) -> &ManagerStats {
        &self.stats
    }

    /// Get message count
    pub async fn message_count(&self) -> usize {
        self.messages.lock().await.len()
    }
}
