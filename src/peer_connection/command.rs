use tokio::sync::oneshot;

#[derive(Debug)]
pub enum CommandType {
    Request(u32),
    Finished,
}

#[derive(Debug)]
pub struct Command {
    pub command: CommandType,
    pub response_channel: oneshot::Sender<u64>,
}

impl Command {
    pub fn new(command: CommandType) -> (Self, oneshot::Receiver<u64>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                command,
                response_channel: tx,
            },
            rx,
        )
    }
}
