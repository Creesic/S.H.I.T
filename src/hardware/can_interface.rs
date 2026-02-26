use async_trait::async_trait;
use crate::core::CanMessage;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;

/// Boxed future type for async operations
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result type for CAN interface operations
pub type CanResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Configuration for a CAN interface
#[derive(Debug, Clone)]
pub struct CanConfig {
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Enable CAN FD mode
    pub fd_mode: bool,
    /// Enable listen-only mode
    pub listen_only: bool,
}

impl Default for CanConfig {
    fn default() -> Self {
        Self {
            bitrate: 500_000,
            fd_mode: false,
            listen_only: false,
        }
    }
}

/// Status of a CAN interface
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanStatus {
    /// Interface is disconnected
    Disconnected,
    /// Interface is connecting
    Connecting,
    /// Interface is connected and ready
    Connected,
    /// Interface has an error
    Error,
}

/// Trait for CAN bus interface implementations
///
/// This trait provides a common interface for different CAN hardware:
/// - USB-CAN serial adapters (SLCAN/Lawicel protocol)
/// - SocketCAN (Linux)
/// - Mock interfaces for testing
#[async_trait]
pub trait CanInterface: Send {
    /// Get the name/identifier of this interface
    fn name(&self) -> &str;

    /// Get the current status of the interface
    fn status(&self) -> CanStatus;

    /// Connect to the CAN interface with the given configuration
    async fn connect(&mut self, config: CanConfig) -> CanResult<()>;

    /// Disconnect from the CAN interface
    async fn disconnect(&mut self) -> CanResult<()>;

    /// Send a CAN message
    async fn send(&mut self, message: &CanMessage) -> CanResult<()>;

    /// Receive a CAN message (non-blocking, returns None if no message available)
    async fn receive(&mut self) -> CanResult<Option<CanMessage>>;

    /// Get the number of messages in the receive buffer
    fn rx_buffer_size(&self) -> usize;

    /// Clear the receive buffer
    fn clear_rx_buffer(&mut self);

    /// Check if the interface supports CAN FD
    fn supports_fd(&self) -> bool {
        false
    }

    /// Get available CAN interfaces on the system
    fn list_interfaces() -> Vec<String> where Self: Sized {
        Vec::new()
    }
}

/// Information about an available CAN interface
#[derive(Debug, Clone)]
pub struct InterfaceInfo {
    /// Interface name/identifier
    pub name: String,
    /// Interface type
    pub interface_type: InterfaceType,
    /// Description
    pub description: Option<String>,
    /// Whether the interface is available for use
    pub available: bool,
}

/// Type of CAN interface
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InterfaceType {
    /// USB-CAN serial adapter
    Serial,
    /// SocketCAN (Linux)
    SocketCan,
    /// Virtual/mock interface
    Virtual,
    /// Unknown type
    Unknown,
}
