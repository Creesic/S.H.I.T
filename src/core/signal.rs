use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A decoded signal value from a CAN message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    /// Signal name (from DBC)
    pub name: String,

    /// Raw CAN message this came from
    pub message_id: u32,

    /// Decoded value
    pub value: f64,

    /// Timestamp
    pub timestamp: DateTime<Utc>,

    /// Units (if specified in DBC)
    pub units: Option<String>,
}

/// Signal value type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignalValue {
    Float(f64),
    Unsigned(u64),
    Signed(i64),
    Boolean(bool),
}

impl SignalValue {
    /// Get as f64 for plotting
    pub fn as_f64(&self) -> f64 {
        match self {
            SignalValue::Float(v) => *v,
            SignalValue::Unsigned(v) => *v as f64,
            SignalValue::Signed(v) => *v as f64,
            SignalValue::Boolean(v) => if *v { 1.0 } else { 0.0 },
        }
    }
}

/// A time series of signal values for plotting
#[derive(Debug, Clone)]
pub struct SignalSeries {
    pub signal_name: String,
    pub data_points: Vec<SignalPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalPoint {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
}
