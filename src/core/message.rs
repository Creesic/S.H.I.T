use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A raw CAN message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanMessage {
    /// Timestamp in UTC
    pub timestamp: DateTime<Utc>,

    /// CAN bus ID (0, 1, 2, etc.)
    pub bus: u8,

    /// CAN message ID (11-bit or 29-bit)
    pub id: u32,

    /// Raw data bytes (0-8 bytes)
    pub data: Vec<u8>,
}

impl CanMessage {
    /// Create a new CAN message
    pub fn new(bus: u8, id: u32, data: Vec<u8>) -> Self {
        Self {
            timestamp: Utc::now(),
            bus,
            id,
            data,
        }
    }

    /// Check if this is an extended (29-bit) CAN ID
    pub fn is_extended(&self) -> bool {
        self.id > 0x7FF
    }

    /// Get data as hex string
    pub fn hex_data(&self) -> String {
        self.data
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Get timestamp as Unix timestamp in seconds
    pub fn timestamp_unix(&self) -> f64 {
        self.timestamp.timestamp_millis() as f64 / 1000.0
    }

    /// Parse hex string to data bytes
    pub fn parse_hex(hex: &str) -> anyhow::Result<Vec<u8>> {
        let hex = hex.replace(' ', "");
        // Strip 0x or 0X prefix if present
        let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(&hex);

        if hex.len() % 2 != 0 {
            anyhow::bail!("Hex string must have even length");
        }

        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("Failed to parse hex: {}", e))
    }
}
