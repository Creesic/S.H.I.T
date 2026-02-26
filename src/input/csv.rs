use anyhow::{Context, Result};
use std::path::Path;
use crate::core::CanMessage;
use chrono::{DateTime, Utc};

/// Load CAN messages from a CSV file
///
/// Supports flexible column formats:
/// - time,bus,msg_id,data
/// - timestamp,can_id,payload
/// - time,id,hex_data
///
/// Timestamps are treated as relative seconds from the start of the log
pub fn load_csv(path: &str) -> Result<Vec<CanMessage>> {
    let file_path = Path::new(path);
    let mut rdr = csv::Reader::from_path(file_path)?;

    let headers = rdr.headers()?;
    let (time_idx, bus_idx, id_idx, data_idx) = detect_columns(headers)?;

    let mut messages = Vec::new();

    // Use a fixed base time for all messages
    let base_time = Utc::now();

    // Debug: log the base time
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/can-viz-csv-debug.txt") {
        let _ = writeln!(f, "Loading CSV: base_time = {}", base_time.format("%H:%M:%S%.3f"));
    }

    for result in rdr.records() {
        let record = result.context("Failed to read CSV row")?;

        // Parse timestamp as relative seconds from log start
        let timestamp = record.get(time_idx).and_then(|s| s.parse::<f64>().ok())
            .map(|relative_secs| {
                // Add relative seconds to base time
                let ms = (relative_secs * 1000.0) as i64;
                base_time + chrono::Duration::milliseconds(ms)
            })
            .unwrap_or_else(|| base_time);

        // Parse bus ID
        let bus = record.get(bus_idx).and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);

        // Parse CAN ID (could be decimal or hex like "0x123")
        let id = record.get(id_idx)
            .and_then(|s| {
                if s.starts_with("0x") || s.starts_with("0X") {
                    u32::from_str_radix(&s[2..], 16).ok()
                } else {
                    s.parse::<u32>().ok()
                }
            })
            .context("Failed to parse CAN ID")?;

        // Parse data bytes
        let hex_data = record.get(data_idx).context("Missing data column")?;
        let data = CanMessage::parse_hex(hex_data)?;

        // Debug: log first few messages
        if messages.len() <= 5 {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/can-viz-csv-debug.txt") {
                let _ = writeln!(f, "  Message {}: time={}, bus={}, id={:04X}, data={:02X?}",
                    messages.len(),
                    timestamp.format("%H:%M:%S%.3f"),
                    bus, id, data);
            }
        }

        messages.push(CanMessage { timestamp, bus, id, data });
    }

    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/can-viz-csv-debug.txt") {
        let _ = writeln!(f, "CSV loaded: {} messages", messages.len());
        if let Some(first) = messages.first() {
            let _ = writeln!(f, "  First message timestamp: {}", first.timestamp.format("%H:%M:%S%.3f"));
        }
        if let Some(last) = messages.last() {
            let _ = writeln!(f, "  Last message timestamp: {}", last.timestamp.format("%H:%M:%S%.3f"));
        }
    }

    Ok(messages)
}

/// Detect column indices from CSV headers
fn detect_columns(headers: &csv::StringRecord) -> Result<(usize, usize, usize, usize)> {
    let time_idx = find_column(headers, &["time", "timestamp", "t", "ts"])?;
    let bus_idx = find_column(headers, &["bus", "channel", "interface"])?;
    let id_idx = find_column(headers, &["id", "addr", "msg_id", "can_id", "message_id"])?;
    let data_idx = find_column(headers, &["data", "payload", "hex", "bytes"])?;

    Ok((time_idx, bus_idx, id_idx, data_idx))
}

/// Find a column by checking possible names
fn find_column(headers: &csv::StringRecord, names: &[&str]) -> Result<usize> {
    for (idx, header) in headers.iter().enumerate() {
        let header_lower = header.to_lowercase();
        if names.iter().any(|&name| header_lower == name) {
            return Ok(idx);
        }
    }

    anyhow::bail!("Could not find column with names: {:?}", names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex() {
        assert_eq!(
            CanMessage::parse_hex("12 34 AB CD").unwrap(),
            vec![0x12, 0x34, 0xAB, 0xCD]
        );
        assert_eq!(
            CanMessage::parse_hex("1234ABCD").unwrap(),
            vec![0x12, 0x34, 0xAB, 0xCD]
        );
    }
}
