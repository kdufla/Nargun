use tokio::sync::{broadcast, mpsc};

pub fn channel() -> (Sender, Receiver) {
    let (broadcast_tx, _) = broadcast::channel(1);

    let (tx, rx) = mpsc::channel(1);
    let waiter = AliveTaskWaiter(rx);

    let s = Sender::new(broadcast_tx.clone(), waiter);
    let r = Receiver::new(broadcast_tx, tx);

    (s, r)
}

pub type HolderStillAlive = mpsc::Sender<()>;
pub struct AliveTaskWaiter(mpsc::Receiver<()>);

impl AliveTaskWaiter {
    pub async fn wait(mut self) {
        let _ = self.0.recv().await;
    }
}

pub struct Sender {
    sender: broadcast::Sender<()>,
    waiter: AliveTaskWaiter,
}

impl Sender {
    fn new(sender: broadcast::Sender<()>, waiter: AliveTaskWaiter) -> Self {
        Self { sender, waiter }
    }

    pub fn send(self) -> AliveTaskWaiter {
        let _ = self.sender.send(());
        self.waiter
    }
}

#[derive(Debug)]
pub struct Receiver {
    sender: broadcast::Sender<()>,
    receiver: broadcast::Receiver<()>,
    alive_marker: HolderStillAlive,
}

impl Receiver {
    fn new(sender: broadcast::Sender<()>, alive_marker: HolderStillAlive) -> Self {
        Self {
            receiver: sender.subscribe(),
            sender,
            alive_marker,
        }
    }

    pub async fn recv(&mut self) -> () {
        self.receiver.recv().await.unwrap()
    }
}

impl Clone for Receiver {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            receiver: self.sender.subscribe(),
            alive_marker: self.alive_marker.clone(),
        }
    }
}
