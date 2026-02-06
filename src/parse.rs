//! Message parsing from gzipped JSONL files.

use flate2::read::GzDecoder;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

pub struct ParsedMessage {
    pub publish_time: i64,
    pub market_changes: Vec<Value>,
}

fn is_gzipped(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext == "gz" || ext == "gzip")
        .unwrap_or(false)
}

pub fn parse_messages(path: &Path) -> Result<Vec<ParsedMessage>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;

    let reader: Box<dyn Read> = if is_gzipped(path) {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    let buf_reader = BufReader::with_capacity(1024 * 1024, reader);
    let mut messages = Vec::new();

    for line_result in buf_reader.lines() {
        let line = line_result.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let publish_time = msg.get("pt").and_then(|v| v.as_i64()).unwrap_or(0);

        let market_changes = if let Some(mc) = msg.get("mc").and_then(|v| v.as_array()) {
            mc.clone()
        } else if msg.get("marketDefinition").is_some() || msg.get("rc").is_some() {
            vec![msg.clone()]
        } else {
            continue;
        };

        if publish_time > 0 {
            messages.push(ParsedMessage {
                publish_time,
                market_changes,
            });
        }
    }

    Ok(messages)
}
