//! Transport-neutral handles for repository requests forwarded to the UI.

use crate::BrowserOpenRequest;

/// Sends requests from a broker thread to the UI event loop.
#[derive(Clone)]
pub struct BrowserRequestSender(smol::channel::Sender<BrowserOpenRequest>);

/// The receiving end consumed by `UiLaunch::browser_requests`.
pub struct BrowserRequestReceiver(pub(crate) smol::channel::Receiver<BrowserOpenRequest>);

/// The UI has stopped accepting repository requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserRequestClosed;

impl std::fmt::Display for BrowserRequestClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the browser request channel is closed")
    }
}

impl std::error::Error for BrowserRequestClosed {}

/// Creates a channel without exposing the host's executor or transport.
pub fn browser_request_channel() -> (BrowserRequestSender, BrowserRequestReceiver) {
    let (sender, receiver) = smol::channel::unbounded();
    (
        BrowserRequestSender(sender),
        BrowserRequestReceiver(receiver),
    )
}

impl BrowserRequestSender {
    /// Queues a request without blocking the broker thread.
    pub fn try_send(&self, request: BrowserOpenRequest) -> Result<(), BrowserRequestClosed> {
        self.0.try_send(request).map_err(|_| BrowserRequestClosed)
    }
}

impl BrowserRequestReceiver {
    /// Waits for the next broker request without exposing an executor type.
    pub async fn recv(&self) -> Result<BrowserOpenRequest, BrowserRequestClosed> {
        self.0.recv().await.map_err(|_| BrowserRequestClosed)
    }

    /// Takes an already queued request, or `None` when there is none.
    pub fn try_recv(&self) -> Option<BrowserOpenRequest> {
        self.0.try_recv().ok()
    }
}
