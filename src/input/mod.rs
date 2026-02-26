pub mod csv;
pub mod rlog;

pub use csv::load_csv;
pub use rlog::load_rlog;

use anyhow::Result;
use crate::core::CanMessage;

/// Input format detection result
#[derive(Debug, Clone)]
pub enum InputFormat {
    Csv,
    Rlog,
    Unknown,
}

/// Detect the format of an input file by checking the file header/magic
pub fn detect_format(data: &[u8]) -> InputFormat {
    // rlog files start with a specific magic bytes pattern
    if is_rlog(data) {
        return InputFormat::Rlog;
    }

    // Check if it looks like CSV (text, comma separated)
    if is_csv(data) {
        return InputFormat::Csv;
    }

    InputFormat::Unknown
}

fn is_rlog(data: &[u8]) -> bool {
    // comma's rlog format starts with "bz" magic
    // This is a simplified check - real implementation would verify the full header
    data.len() >= 2 && data[0] == b'b' && data[1] == b'z'
}

fn is_csv(data: &[u8]) -> bool {
    // Check if the data looks like CSV (text with commas)
    // Look for a line with commas in the first 500 bytes
    if data.len() < 10 {
        return false;
    }

    let sample = std::str::from_utf8(&data[..data.len().min(500)]);
    match sample {
        Ok(text) => {
            // Check for CSV-like patterns (multiple commas on a line)
            text.lines().take(5).any(|line| line.chars().filter(|&c| c == ',').count() >= 2)
        }
        Err(_) => false,
    }
}

/// Load CAN data from a file, auto-detecting format
pub fn load_file(path: &str) -> Result<Vec<CanMessage>> {
    let data = std::fs::read(path)?;

    match detect_format(&data) {
        InputFormat::Csv => load_csv(path),
        InputFormat::Rlog => load_rlog(path),
        InputFormat::Unknown => anyhow::bail!("Unknown input format"),
    }
}
