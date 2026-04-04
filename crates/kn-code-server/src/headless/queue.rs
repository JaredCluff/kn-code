use std::collections::VecDeque;
use tokio::sync::Notify;

const MAX_QUEUE_SIZE: usize = 10000;

pub struct CommandQueue {
    commands: VecDeque<QueuedCommand>,
    notify: Notify,
}

#[derive(Debug, Clone)]
pub enum QueuedCommand {
    GrantPermission {
        request_id: String,
    },
    DenyPermission {
        request_id: String,
        message: Option<String>,
    },
    Cancel,
    Message {
        content: String,
    },
    SetMode {
        mode: String,
    },
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandQueue {
    pub fn new() -> Self {
        Self {
            commands: VecDeque::new(),
            notify: Notify::new(),
        }
    }

    pub fn push(&mut self, command: QueuedCommand) -> Result<(), &'static str> {
        if self.commands.len() >= MAX_QUEUE_SIZE {
            return Err("Command queue is full");
        }
        self.commands.push_back(command);
        self.notify.notify_one();
        Ok(())
    }

    pub async fn next(&mut self) -> QueuedCommand {
        loop {
            if let Some(cmd) = self.commands.pop_front() {
                return cmd;
            }
            self.notify.notified().await;
        }
    }

    pub fn try_next(&mut self) -> Option<QueuedCommand> {
        self.commands.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        !self.commands.is_empty()
    }

    pub fn clear(&mut self) {
        self.commands.clear();
    }

    pub fn has_cancel(&self) -> bool {
        self.commands
            .iter()
            .any(|cmd| matches!(cmd, QueuedCommand::Cancel))
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}
