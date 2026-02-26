use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use anyhow::{Context, Result};

/// Represents a loaded DBC file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbcFile {
    /// Version string
    pub version: String,
    /// All messages in the DBC
    pub messages: Vec<DbcMessage>,
    /// Quick lookup by CAN ID
    pub message_lookup: HashMap<u32, DbcMessage>,
    /// All value tables (enums)
    pub value_tables: HashMap<String, Vec<ValueDescription>>,
    /// File path (if loaded from file)
    #[serde(skip)]
    pub file_path: Option<String>,
}

impl DbcFile {
    pub fn new() -> Self {
        Self {
            version: String::new(),
            messages: Vec::new(),
            message_lookup: HashMap::new(),
            value_tables: HashMap::new(),
            file_path: None,
        }
    }

    /// Load a DBC file from disk
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read DBC file: {:?}", path))?;

        let mut dbc = Self::parse(&content)?;
        dbc.file_path = Some(path.to_string_lossy().to_string());
        Ok(dbc)
    }

    /// Parse DBC file content
    pub fn parse(content: &str) -> Result<Self> {
        let mut dbc = Self::new();

        // Simple DBC parser - handles basic DBC format
        // For full DBC support, we would use the can-dbc crate
        for line in content.lines() {
            let line = line.trim();

            if line.starts_with("VERSION") {
                dbc.version = line.strip_prefix("VERSION ")
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_string();
            }
            else if line.starts_with("BO_ ") {
                if let Some(msg) = parse_message_line(line) {
                    dbc.message_lookup.insert(msg.id, msg.clone());
                    dbc.messages.push(msg);
                }
            }
            else if line.starts_with("SG_ ") {
                // Signal belonging to last message
                if let Some(msg) = dbc.messages.last_mut() {
                    if let Some(signal) = parse_signal_line(line) {
                        msg.signals.push(signal);
                    }
                }
            }
            else if line.starts_with("VAL_ ") {
                // Value description (enum)
                if let Some((name, values)) = parse_val_line(line) {
                    dbc.value_tables.insert(name, values);
                }
            }
        }

        // Rebuild message lookup after parsing
        dbc.message_lookup = dbc.messages.iter()
            .map(|m| (m.id, m.clone()))
            .collect();

        Ok(dbc)
    }

    /// Save DBC file to disk
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let content = self.to_dbc_string();
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write DBC file: {:?}", path))?;
        Ok(())
    }

    /// Convert to DBC file format string
    pub fn to_dbc_string(&self) -> String {
        let mut output = String::new();

        // Version
        output.push_str(&format!("VERSION \"{}\"\n\n", self.version));

        // Symbols
        output.push_str("NS_ :\n");
        output.push_str("\tNS_DESC_\n");
        output.push_str("\tCM_\n");
        output.push_str("\tBA_DEF_\n");
        output.push_str("\tBA_\n");
        output.push_str("\tVAL_\n");
        output.push_str("\tCAT_DEF_\n");
        output.push_str("\tCAT_\n");
        output.push_str("\tFILTER\n");
        output.push_str("\tBA_DEF_DEF_\n");
        output.push_str("\tEV_DATA_\n");
        output.push_str("\tENVVAR_DATA_\n");
        output.push_str("\tSGTYPE_\n");
        output.push_str("\tSGTYPE_VAL_\n");
        output.push_str("\tBA_DEF_SGTYPE_\n");
        output.push_str("\tBA_SGTYPE_\n");
        output.push_str("\tSIG_TYPE_REF_\n");
        output.push_str("\tVAL_TABLE_\n");
        output.push_str("\tSIG_GROUP_\n");
        output.push_str("\tSIG_VALTYPE_\n");
        output.push_str("\tSIGTYPE_VALTYPE_\n");
        output.push_str("\tBO_TX_BU_\n");
        output.push_str("\tBA_DEF_REL_\n");
        output.push_str("\tBA_REL_\n");
        output.push_str("\tBA_DEF_DEF_REL_\n");
        output.push_str("\tBU_SG_REL_\n");
        output.push_str("\tBU_EV_REL_\n");
        output.push_str("\tBU_BO_REL_\n");
        output.push_str("\tSG_MUL_VAL_\n");
        output.push_str("\n");

        // Bit timing
        output.push_str("BS_:\n\n");

        // Nodes (placeholder)
        output.push_str("BU_: Vector__XXX\n\n");

        // Messages
        for msg in &self.messages {
            output.push_str(&format!(
                "BO_ {} {}: {} Vector__XXX\n",
                msg.id, msg.name, msg.size
            ));
            for signal in &msg.signals {
                let byte_order = match signal.byte_order {
                    ByteOrder::Motorola => '0',
                    ByteOrder::Intel => '1',
                };
                let value_type = match signal.value_type {
                    ValueType::Signed => '-',
                    ValueType::Unsigned => '+',
                };
                output.push_str(&format!(
                    " SG_ {} : {}|{}@{}{} ({},{}) [{}|{}] \"{}\" Vector__XXX\n",
                    signal.name,
                    signal.start_bit,
                    signal.bit_length,
                    byte_order,
                    value_type,
                    signal.factor,
                    signal.offset,
                    signal.minimum.unwrap_or(0.0),
                    signal.maximum.unwrap_or(0.0),
                    signal.unit.as_deref().unwrap_or("")
                ));
            }
            output.push_str("\n");
        }

        // Value tables
        for (name, values) in &self.value_tables {
            output.push_str(&format!("VAL_ {} ", name));
            for val in values {
                output.push_str(&format!("{} \"{}\" ", val.value, val.description));
            }
            output.push_str(";\n");
        }

        output
    }

    /// Add a message to the DBC
    pub fn add_message(&mut self, message: DbcMessage) {
        self.message_lookup.insert(message.id, message.clone());
        self.messages.push(message);
    }

    /// Get a message by CAN ID
    pub fn get_message(&self, id: u32) -> Option<&DbcMessage> {
        self.message_lookup.get(&id)
    }

    /// Get a mutable reference to a message by CAN ID
    pub fn get_message_mut(&mut self, id: u32) -> Option<&mut DbcMessage> {
        self.message_lookup.get_mut(&id)
    }

    /// Remove a message by CAN ID
    pub fn remove_message(&mut self, id: u32) -> Option<DbcMessage> {
        let msg = self.message_lookup.remove(&id);
        if let Some(ref msg) = msg {
            self.messages.retain(|m| m.id != id);
        }
        msg
    }

    /// Get all message IDs
    pub fn message_ids(&self) -> Vec<u32> {
        self.messages.iter().map(|m| m.id).collect()
    }

    /// Check if the DBC has any messages
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

impl Default for DbcFile {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a message line from DBC format
/// Format: BO_ <id> <name>: <dlc> <transmitter>
fn parse_message_line(line: &str) -> Option<DbcMessage> {
    // BO_ 123 MessageName: 8 Vector__XXX
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 || parts[0] != "BO_" {
        return None;
    }

    let id = parts[1].parse::<u32>().ok()?;
    let name = parts[2].trim_end_matches(':').to_string();
    let size = parts[3].parse::<u8>().ok()?;

    Some(DbcMessage {
        id,
        name,
        size,
        signals: Vec::new(),
    })
}

/// Parse a signal line from DBC format
/// Format: SG_ <name> [M|m<val>] : <start_bit>|<bit_length>@<byte_order><value_type> (<factor>,<offset>) [<min>|<max>] "<unit>" <receiver>
fn parse_signal_line(line: &str) -> Option<DbcSignal> {
    // SG_ SignalName : 0|8@1+ (1,0) [0|255] "units" Vector__XXX
    // SG_ SignalName M : 0|8@1+ (1,0) [0|255] "units" Vector__XXX  (multiplexed)
    let line = line.strip_prefix("SG_ ")?;

    // Find the colon - everything before it is name (and optional multiplexer)
    let colon_pos = line.find(':')?;
    let name_part = &line[..colon_pos];
    let rest = &line[colon_pos + 1..];

    // Extract signal name (first token before any multiplexer indicator)
    let name = name_part.split_whitespace().next()?.to_string();

    // Parse the rest: start|len@order+ (factor,offset) [min|max] "unit" receiver
    let rest = rest.trim_start();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // Parse bit position and length: "0|8@1+" or just "0|8"
    let bit_info = parts[0];

    // Find the @ symbol to split bit info from byte order
    let at_pos = bit_info.find('@')?;
    let bit_part = &bit_info[..at_pos];
    let order_type = &bit_info[at_pos..];  // Include the @ for parse_order_and_type

    let bit_parts: Vec<&str> = bit_part.split('|').collect();
    if bit_parts.len() != 2 {
        return None;
    }
    let start_bit = bit_parts[0].parse::<u8>().ok()?;
    let bit_length = bit_parts[1].parse::<u8>().ok()?;

    // Parse byte order and type
    let (byte_order, value_type) = parse_order_and_type(order_type)?;

    // Find factor/offset in parentheses
    let mut factor: f64 = 1.0;
    let mut offset: f64 = 0.0;
    for part in &parts {
        if part.starts_with('(') {
            let fo = part.trim_matches(|c| c == '(' || c == ')');
            let fo_parts: Vec<&str> = fo.split(',').collect();
            if fo_parts.len() == 2 {
                factor = fo_parts[0].parse().ok()?;
                offset = fo_parts[1].parse().ok()?;
            }
            break;
        }
    }

    // Parse min and max: "[0|255]"
    let (minimum, maximum) = parts.iter()
        .find(|p| p.starts_with('['))
        .and_then(|p| Some(parse_min_max(p)))
        .unwrap_or((None, None));

    // Parse unit: "\"units\""
    let unit = parts.iter()
        .find(|p| p.starts_with('"'))
        .map(|p| p.trim_matches('"').to_string());

    Some(DbcSignal {
        name,
        start_bit,
        bit_length,
        byte_order,
        value_type,
        factor,
        offset,
        minimum,
        maximum,
        unit,
        multiplexor: None,
    })
}

/// Parse byte order and value type from format like "@1+"
fn parse_order_and_type(s: &str) -> Option<(ByteOrder, ValueType)> {
    if !s.starts_with('@') || s.len() < 3 {
        return None;
    }

    let byte_order = match s.chars().nth(1)? {
        '0' => ByteOrder::Motorola,
        '1' => ByteOrder::Intel,
        _ => return None,
    };

    let value_type = match s.chars().nth(2)? {
        '+' => ValueType::Unsigned,
        '-' => ValueType::Signed,
        _ => return None,
    };

    Some((byte_order, value_type))
}

/// Parse min and max from format like "[0|255]"
fn parse_min_max(s: &str) -> (Option<f64>, Option<f64>) {
    let s = s.trim_matches(|c| c == '[' || c == ']');
    let parts: Vec<&str> = s.split('|').collect();

    if parts.len() != 2 {
        return (None, None);
    }

    let min = parts[0].parse::<f64>().ok();
    let max = parts[1].parse::<f64>().ok();

    (min, max)
}

/// Parse a VAL line (value descriptions/enums)
/// Format: VAL_ <id> <signal_name> <value1> "<description1>" <value2> "<description2>" ;
fn parse_val_line(line: &str) -> Option<(String, Vec<ValueDescription>)> {
    let line = line.strip_prefix("VAL_ ")?;
    let parts: Vec<&str> = line.split('"').collect();

    if parts.len() < 3 {
        return None;
    }

    // First part contains ID and signal name, and the first value
    let first_parts: Vec<&str> = parts[0].split_whitespace().collect();
    if first_parts.len() < 3 {
        return None;
    }

    let signal_name = first_parts[1].to_string();
    let mut values = Vec::new();

    // Parse value-description pairs
    let mut i = 0;
    while i + 1 < parts.len() {
        // Get the value number from before the quote
        let value_part = parts[i].trim();
        let value = value_part.split_whitespace().last()
            .and_then(|s| s.parse::<i64>().ok());

        // Get the description from between quotes
        if i + 1 < parts.len() {
            let description = parts[i + 1].to_string();
            if let Some(v) = value {
                values.push(ValueDescription { value: v, description });
            }
        }
        i += 2;
    }

    if values.is_empty() {
        return None;
    }

    Some((signal_name, values))
}

/// A CAN message defined in the DBC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbcMessage {
    /// CAN message ID (11-bit or 29-bit)
    pub id: u32,
    /// Message name
    pub name: String,
    /// Data Length Code (DLC), 0-8
    pub size: u8,
    /// Signals contained in this message
    pub signals: Vec<DbcSignal>,
}

impl DbcMessage {
    /// Create a new DBC message definition
    pub fn new(id: u32, name: &str, size: u8) -> Self {
        Self {
            id,
            name: name.to_string(),
            size,
            signals: Vec::new(),
        }
    }

    /// Add a signal to this message
    pub fn add_signal(&mut self, signal: DbcSignal) {
        self.signals.push(signal);
    }

    /// Get a signal by name
    pub fn get_signal(&self, name: &str) -> Option<&DbcSignal> {
        self.signals.iter().find(|s| s.name == name)
    }

    /// Get a mutable signal by name
    pub fn get_signal_mut(&mut self, name: &str) -> Option<&mut DbcSignal> {
        self.signals.iter_mut().find(|s| s.name == name)
    }

    /// Check for signal overlap (validation)
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Check DLC
        if self.size > 8 {
            errors.push(format!("Message {} has invalid DLC: {}", self.name, self.size));
        }

        // Check for signal overlap
        for i in 0..self.signals.len() {
            for j in (i + 1)..self.signals.len() {
                if signals_overlap(&self.signals[i], &self.signals[j]) {
                    errors.push(format!(
                        "Signals '{}' and '{}' overlap in message {}",
                        self.signals[i].name, self.signals[j].name, self.name
                    ));
                }
            }
        }

        // Check each signal fits within the message
        for signal in &self.signals {
            let end_bit = signal.start_bit as usize + signal.bit_length as usize;
            let max_bits = self.size as usize * 8;
            if end_bit > max_bits {
                errors.push(format!(
                    "Signal '{}' extends beyond message boundary ({} > {})",
                    signal.name, end_bit, max_bits
                ));
            }
        }

        errors
    }
}

/// A signal defined in the DBC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbcSignal {
    /// Signal name
    pub name: String,
    /// Starting bit position (in DBC notation)
    pub start_bit: u8,
    /// Number of bits
    pub bit_length: u8,
    /// Byte order (Motorola = big endian, Intel = little endian)
    pub byte_order: ByteOrder,
    /// Value type (signed or unsigned)
    pub value_type: ValueType,
    /// Factor for scaling
    pub factor: f64,
    /// Offset for scaling
    pub offset: f64,
    /// Minimum physical value
    pub minimum: Option<f64>,
    /// Maximum physical value
    pub maximum: Option<f64>,
    /// Unit string
    pub unit: Option<String>,
    /// Multiplexor configuration (if this is a multiplexed signal)
    pub multiplexor: Option<Multiplexor>,
}

impl DbcSignal {
    /// Create a new unsigned Intel (little-endian) signal
    pub fn new(name: &str, start_bit: u8, bit_length: u8) -> Self {
        Self {
            name: name.to_string(),
            start_bit,
            bit_length,
            byte_order: ByteOrder::Intel,
            value_type: ValueType::Unsigned,
            factor: 1.0,
            offset: 0.0,
            minimum: None,
            maximum: None,
            unit: None,
            multiplexor: None,
        }
    }

    /// Create a signal with full options
    pub fn with_options(
        name: &str,
        start_bit: u8,
        bit_length: u8,
        byte_order: ByteOrder,
        value_type: ValueType,
        factor: f64,
        offset: f64,
    ) -> Self {
        Self {
            name: name.to_string(),
            start_bit,
            bit_length,
            byte_order,
            value_type,
            factor,
            offset,
            minimum: None,
            maximum: None,
            unit: None,
            multiplexor: None,
        }
    }

    /// Set the unit
    pub fn with_unit(mut self, unit: &str) -> Self {
        self.unit = Some(unit.to_string());
        self
    }

    /// Set the range
    pub fn with_range(mut self, min: f64, max: f64) -> Self {
        self.minimum = Some(min);
        self.maximum = Some(max);
        self
    }

    /// Get the raw value range (before factor/offset)
    pub fn raw_range(&self) -> (u64, u64) {
        let max_raw = (1u64 << self.bit_length) - 1;
        (0, max_raw)
    }

    /// Get the physical value range (after factor/offset)
    pub fn physical_range(&self) -> (f64, f64) {
        let (raw_min, raw_max) = self.raw_range();
        let physical_min = (raw_min as f64) * self.factor + self.offset;
        let physical_max = (raw_max as f64) * self.factor + self.offset;
        (physical_min, physical_max)
    }
}

/// Byte order for signal encoding
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ByteOrder {
    /// Big endian (Motorola format)
    Motorola,
    /// Little endian (Intel format)
    Intel,
}

impl Default for ByteOrder {
    fn default() -> Self {
        ByteOrder::Intel
    }
}

/// Value type for signals
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ValueType {
    /// Signed integer (two's complement)
    Signed,
    /// Unsigned integer
    Unsigned,
}

impl Default for ValueType {
    fn default() -> Self {
        ValueType::Unsigned
    }
}

/// Multiplexor configuration for multiplexed signals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Multiplexor {
    /// This signal is the multiplexor selector
    Signal,
    /// This signal appears when the multiplexor has this value
    Value(u8),
}

/// Value description for enum-like signals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueDescription {
    /// Numeric value
    pub value: i64,
    /// Human-readable description
    pub description: String,
}

/// Check if two signals overlap in bit positions
fn signals_overlap(a: &DbcSignal, b: &DbcSignal) -> bool {
    let a_start = a.start_bit as usize;
    let a_end = a_start + a.bit_length as usize;
    let b_start = b.start_bit as usize;
    let b_end = b_start + b.bit_length as usize;

    // Simple overlap check - doesn't account for byte order differences
    a_start < b_end && b_start < a_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_line() {
        let line = "BO_ 256 StatusMessage: 8 Vector__XXX";
        let msg = parse_message_line(line).unwrap();
        assert_eq!(msg.id, 256);
        assert_eq!(msg.name, "StatusMessage");
        assert_eq!(msg.size, 8);
    }

    #[test]
    fn test_parse_signal_line() {
        let line = "SG_ Speed : 0|16@1+ (0.1,0) [0|6553.5] \"km/h\" Vector__XXX";
        let signal = parse_signal_line(line).unwrap();
        assert_eq!(signal.name, "Speed");
        assert_eq!(signal.start_bit, 0);
        assert_eq!(signal.bit_length, 16);
        assert_eq!(signal.byte_order, ByteOrder::Intel);
        assert_eq!(signal.value_type, ValueType::Unsigned);
        assert_eq!(signal.factor, 0.1);
        assert_eq!(signal.offset, 0.0);
        assert_eq!(signal.unit, Some("km/h".to_string()));
    }

    #[test]
    fn test_dbc_roundtrip() {
        let mut dbc = DbcFile::new();
        dbc.version = "1.0".to_string();

        let mut msg = DbcMessage::new(0x100, "TestMessage", 8);
        msg.add_signal(DbcSignal::with_options(
            "Signal1", 0, 8, ByteOrder::Intel, ValueType::Unsigned, 1.0, 0.0
        ));
        dbc.add_message(msg);

        let output = dbc.to_dbc_string();
        let parsed = DbcFile::parse(&output).unwrap();

        assert_eq!(parsed.version, "1.0");
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].id, 0x100);
        assert_eq!(parsed.messages[0].signals.len(), 1);
    }

    #[test]
    fn test_message_validation() {
        let mut msg = DbcMessage::new(0x100, "Test", 8);

        // Add overlapping signals
        msg.add_signal(DbcSignal::new("Sig1", 0, 16));
        msg.add_signal(DbcSignal::new("Sig2", 8, 16)); // Overlaps with Sig1

        let errors = msg.validate();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("overlap"));
    }
}
