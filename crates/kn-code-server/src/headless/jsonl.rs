use crate::headless::events::SdkEvent;
use std::io::{self, Write};
use tokio::sync::mpsc;

pub struct JsonlEmitter {
    tx: mpsc::UnboundedSender<String>,
}

impl JsonlEmitter {
    pub fn new() -> (Self, JsonlReceiver) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, JsonlReceiver { rx })
    }

    pub fn emit(&self, event: &SdkEvent) {
        if let Ok(json) = serde_json::to_string(event) {
            let _ = self.tx.send(json);
        }
    }

    pub fn emit_batch(&self, events: &[SdkEvent]) {
        let json_lines: Vec<String> = events
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect();
        for line in json_lines {
            let _ = self.tx.send(line);
        }
    }
}

impl Default for JsonlEmitter {
    fn default() -> Self {
        Self::new().0
    }
}

pub struct JsonlReceiver {
    rx: mpsc::UnboundedReceiver<String>,
}

impl JsonlReceiver {
    pub async fn drain_to_stdout(&mut self) {
        let mut stdout = io::stdout();
        while let Some(line) = self.rx.recv().await {
            let _ = writeln!(stdout, "{}", line);
            let _ = stdout.flush();
        }
    }

    pub async fn collect(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(line) = self.rx.recv().await {
            lines.push(line);
        }
        lines
    }
}

pub struct AtomicStdoutWriter;

impl AtomicStdoutWriter {
    pub fn write_event(event: &SdkEvent) -> io::Result<()> {
        let json = serde_json::to_string(event).map_err(io::Error::other)?;
        let mut stdout = io::stdout();
        writeln!(stdout, "{}", json)?;
        stdout.flush()
    }

    pub fn write_batch(events: &[SdkEvent]) -> io::Result<()> {
        let mut buffer = String::new();
        for event in events {
            let json = serde_json::to_string(event).map_err(io::Error::other)?;
            buffer.push_str(&json);
            buffer.push('\n');
        }
        let mut stdout = io::stdout();
        stdout.write_all(buffer.as_bytes())?;
        stdout.flush()
    }
}
