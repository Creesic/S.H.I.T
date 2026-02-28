//! Collection for managing multiple CAN interfaces simultaneously
//!
//! Each interface is assigned a unique bus ID (0, 1, 2, ...) and
//! messages from all interfaces are aggregated with their bus IDs preserved.
//!
//! Bus IDs are reused when interfaces disconnect - the lowest available ID is always assigned.

use crate::hardware::can_manager::{CanManager, ConnectionStatus, ManagerMessage, ManagerStats};
use crate::hardware::can_interface::{CanConfig, InterfaceType};
use std::collections::{HashMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// A managed CAN interface with its assigned bus ID
pub struct ManagedInterface {
    /// Bus ID assigned to this interface
    pub bus_id: u8,
    /// The CAN manager for this interface
    pub manager: CanManager,
    /// Interface name (e.g., "/dev/ttyUSB0")
    pub interface_name: String,
    /// Type of interface
    pub interface_type: InterfaceType,
}

/// Statistics for a specific interface
#[derive(Clone)]
pub struct InterfaceStats {
    /// Bus ID for this interface
    pub bus_id: u8,
    /// Interface name
    pub interface_name: String,
    /// Current connection status
    pub status: ConnectionStatus,
    /// Number of messages received
    pub messages_received: u64,
    /// Number of messages sent
    pub messages_sent: u64,
    /// Number of errors
    pub errors: u64,
}

/// Bus ID allocator that reuses freed IDs
struct BusIdAllocator {
    /// Available bus IDs (sorted, always get the lowest)
    available: BTreeSet<u8>,
    /// Next bus ID to assign if no freed IDs available
    next_id: u8,
}

impl BusIdAllocator {
    fn new() -> Self {
        Self {
            available: BTreeSet::new(),
            next_id: 0,
        }
    }

    /// Allocate the next available bus ID
    fn allocate(&mut self) -> u8 {
        // First try to reuse a freed ID (lowest available)
        if let Some(&id) = self.available.iter().next() {
            self.available.remove(&id);
            id
        } else {
            // No freed IDs, allocate the next one
            let id = self.next_id;
            self.next_id = id.wrapping_add(1);
            id
        }
    }

    /// Free a bus ID so it can be reused
    fn free(&mut self, bus_id: u8) {
        self.available.insert(bus_id);
    }

    /// Free all bus IDs (reset allocator)
    fn free_all(&mut self) {
        self.available.clear();
        self.next_id = 0;
    }
}

/// Collection managing multiple CAN interfaces
///
/// Each interface gets a unique bus ID (0, 1, 2, ...)
/// Bus IDs are reused when interfaces disconnect.
/// Messages from all interfaces are aggregated.
pub struct CanManagerCollection {
    /// Map of bus_id to managed interface
    interfaces: Arc<RwLock<HashMap<u8, ManagedInterface>>>,
    /// Bus ID allocator
    allocator: Arc<Mutex<BusIdAllocator>>,
}

impl CanManagerCollection {
    /// Create a new empty collection
    pub fn new() -> Self {
        Self {
            interfaces: Arc::new(RwLock::new(HashMap::new())),
            allocator: Arc::new(Mutex::new(BusIdAllocator::new())),
        }
    }

    /// Check if an interface is already connected or connecting
    pub async fn has_interface(&self, interface: &str) -> bool {
        let interfaces = self.interfaces.read().await;
        interfaces.values().any(|m| m.interface_name == interface)
    }

    /// Connect to a new CAN interface, assigning it the lowest available bus ID
    ///
    /// Returns the assigned bus ID on success
    pub async fn connect(
        &self,
        interface: &str,
        config: CanConfig,
        interface_type: InterfaceType,
    ) -> Result<u8, String> {
        // Prevent duplicate connection to same interface
        if self.has_interface(interface).await {
            return Err(format!("Already connected or connecting to {}", interface));
        }

        // Allocate the lowest available bus ID
        let bus_id = {
            let mut allocator = self.allocator.lock().await;
            allocator.allocate()
        };

        // Create new manager for this interface
        let mut manager = CanManager::new();

        // Connect using the bus ID
        match manager.connect_with_bus(interface, config, interface_type, bus_id).await {
            Ok(()) => {
                // Store the interface
                let managed = ManagedInterface {
                    bus_id,
                    manager,
                    interface_name: interface.to_string(),
                    interface_type,
                };

                self.interfaces.write().await.insert(bus_id, managed);
                Ok(bus_id)
            }
            Err(e) => {
                // Connection failed, free the bus ID
                self.allocator.lock().await.free(bus_id);
                Err(e)
            }
        }
    }

    /// Disconnect a specific interface by bus ID
    pub async fn disconnect(&self, bus_id: u8) -> Result<(), String> {
        let mut interfaces = self.interfaces.write().await;
        if let Some(mut managed) = interfaces.remove(&bus_id) {
            managed.manager.disconnect().await;
            // Free the bus ID for reuse
            self.allocator.lock().await.free(bus_id);
            Ok(())
        } else {
            Err(format!("No interface with bus ID {}", bus_id))
        }
    }

    /// Disconnect all interfaces
    pub async fn disconnect_all(&self) {
        let mut interfaces = self.interfaces.write().await;
        let mut allocator = self.allocator.lock().await;

        for (bus_id, mut managed) in interfaces.drain() {
            let _ = managed.manager.disconnect().await;
            allocator.free(bus_id);
        }
    }

    /// Get all messages from all interfaces and clear their buffers
    ///
    /// Messages are sorted by timestamp for consistent ordering
    pub async fn get_messages(&self) -> Vec<ManagerMessage> {
        let mut all_messages = Vec::new();
        let interfaces = self.interfaces.read().await;

        for (_, managed) in interfaces.iter() {
            let msgs = managed.manager.get_messages().await;
            all_messages.extend(msgs);
        }

        // Sort by timestamp for consistent ordering
        all_messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        all_messages
    }

    /// Get list of connected interfaces with their status
    pub async fn list_interfaces(&self) -> Vec<(u8, String, ConnectionStatus)> {
        let interfaces = self.interfaces.read().await;
        let mut result = Vec::new();

        for (_, managed) in interfaces.iter() {
            let status = managed.manager.status().await;
            result.push((managed.bus_id, managed.interface_name.clone(), status));
        }

        // Sort by bus_id
        result.sort_by_key(|(bus_id, _, _)| *bus_id);
        result
    }

    /// Get statistics for all interfaces
    pub async fn get_stats(&self) -> Vec<InterfaceStats> {
        let interfaces = self.interfaces.read().await;
        let mut stats = Vec::new();

        for (_, managed) in interfaces.iter() {
            let status = managed.manager.status().await;
            let manager_stats = managed.manager.get_stats();

            stats.push(InterfaceStats {
                bus_id: managed.bus_id,
                interface_name: managed.interface_name.clone(),
                status,
                messages_received: manager_stats.messages_received.load(std::sync::atomic::Ordering::SeqCst),
                messages_sent: manager_stats.messages_sent.load(std::sync::atomic::Ordering::SeqCst),
                errors: manager_stats.errors.load(std::sync::atomic::Ordering::SeqCst),
            });
        }

        // Sort by bus_id
        stats.sort_by_key(|s| s.bus_id);
        stats
    }

    /// Send a message to a specific bus
    pub async fn send_to_bus(&self, bus_id: u8, message: crate::core::CanMessage) -> Result<(), String> {
        let interfaces = self.interfaces.read().await;
        if let Some(managed) = interfaces.get(&bus_id) {
            managed.manager.send(message).await
        } else {
            Err(format!("No interface with bus ID {}", bus_id))
        }
    }

    /// Get the number of connected interfaces
    pub async fn interface_count(&self) -> usize {
        self.interfaces.read().await.len()
    }

    /// Check if any interface is connected
    pub async fn has_active_connection(&self) -> bool {
        let interfaces = self.interfaces.read().await;
        for (_, managed) in interfaces.iter() {
            if matches!(managed.manager.status().await, ConnectionStatus::Connected) {
                return true;
            }
        }
        false
    }
}

impl Default for CanManagerCollection {
    fn default() -> Self {
        Self::new()
    }
}
