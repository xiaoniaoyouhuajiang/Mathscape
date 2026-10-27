//! Round trace persistence: one [`AgentEvent`] per line (JSONL).

use std::io::{self, BufRead, Write};

use crate::AgentEvent;

#[derive(Debug, thiserror::Error)]
pub enum JsonlError {
    #[error("I/O error while reading or writing agent events")]
    Io(#[from] io::Error),
    #[error("invalid agent event on line {line}")]
    Parse {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize agent event")]
    Serialize(#[source] serde_json::Error),
}

/// Serializes one event as a single line without the trailing newline.
pub fn to_jsonl_line(event: &AgentEvent) -> Result<String, JsonlError> {
    serde_json::to_string(event).map_err(JsonlError::Serialize)
}

pub fn from_jsonl_line(line: &str) -> Result<AgentEvent, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn write_jsonl<'a, W: Write>(
    mut writer: W,
    events: impl IntoIterator<Item = &'a AgentEvent>,
) -> Result<(), JsonlError> {
    for event in events {
        serde_json::to_writer(&mut writer, event).map_err(JsonlError::Serialize)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

/// Reads all events; blank lines are skipped, line numbers are 1-based.
pub fn read_jsonl<R: BufRead>(reader: R) -> Result<Vec<AgentEvent>, JsonlError> {
    let mut events = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        events.push(from_jsonl_line(&line).map_err(|source| JsonlError::Parse {
            line: index + 1,
            source,
        })?);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentBackend, RoundId, scripted_round};

    #[test]
    fn scripted_round_trips_through_jsonl() {
        let events = scripted_round(&RoundId::parse("r-jsonl").unwrap(), AgentBackend::Custom);
        let mut buffer = Vec::new();
        write_jsonl(&mut buffer, &events).unwrap();
        let text = String::from_utf8(buffer.clone()).unwrap();
        assert_eq!(text.lines().count(), events.len());
        assert!(text.ends_with('\n'));
        let back = read_jsonl(buffer.as_slice()).unwrap();
        assert_eq!(back, events);
        for event in &events {
            assert_eq!(
                &from_jsonl_line(&to_jsonl_line(event).unwrap()).unwrap(),
                event
            );
        }
    }

    #[test]
    fn reports_bad_line_number() {
        let events = scripted_round(&RoundId::parse("r").unwrap(), AgentBackend::Codex);
        let mut text = String::new();
        text.push_str(&to_jsonl_line(&events[0]).unwrap());
        text.push_str("\n\n{not json}\n");
        match read_jsonl(text.as_bytes()) {
            Err(JsonlError::Parse { line, .. }) => assert_eq!(line, 3),
            other => panic!("unexpected {other:?}"),
        }
    }
}
