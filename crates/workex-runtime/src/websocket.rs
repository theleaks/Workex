//! WebSocket support — persistent bidirectional connections.
//! Workers API: `const pair = new WebSocketPair(); return new Response(null, { status: 101, webSocket: pair[1] });`

use crate::headers::Headers;
use crate::request::WorkexRequest;
use crate::response::WorkexResponse;
use bytes::Bytes;

/// WebSocket message types.
#[derive(Debug, Clone, PartialEq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Close,
    Ping,
    Pong,
}

/// A WebSocket connection (server side).
pub struct WorkexWebSocket {
    tx: tokio::sync::mpsc::Sender<WsMessage>,
    rx: tokio::sync::mpsc::Receiver<WsMessage>,
}

/// A WebSocket pair (client + server), matching Workers WebSocketPair.
pub struct WebSocketPair {
    pub client: WorkexWebSocket,
    pub server: WorkexWebSocket,
}

impl WebSocketPair {
    /// Create a new WebSocket pair.
    pub fn new() -> Self {
        let (c_tx, s_rx) = tokio::sync::mpsc::channel(32);
        let (s_tx, c_rx) = tokio::sync::mpsc::channel(32);
        Self {
            client: WorkexWebSocket { tx: c_tx, rx: c_rx },
            server: WorkexWebSocket { tx: s_tx, rx: s_rx },
        }
    }
}

impl WorkexWebSocket {
    /// Check if a request is a WebSocket upgrade.
    pub fn is_upgrade(req: &WorkexRequest) -> bool {
        req.headers.get("upgrade").map(|v| v.eq_ignore_ascii_case("websocket")).unwrap_or(false)
    }

    /// Accept a WebSocket upgrade — returns the 101 response.
    pub fn accept(req: &WorkexRequest) -> Option<(WebSocketPair, WorkexResponse)> {
        if !Self::is_upgrade(req) {
            return None;
        }

        let pair = WebSocketPair::new();
        let mut headers = Headers::new();
        headers.set("upgrade", "websocket");
        headers.set("connection", "upgrade");

        let resp = WorkexResponse::with_init(Bytes::new(), 101, headers);
        Some((pair, resp))
    }

    /// Send a message.
    pub async fn send(&self, msg: WsMessage) -> anyhow::Result<()> {
        self.tx.send(msg).await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Receive a message.
    pub async fn recv(&mut self) -> Option<WsMessage> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_websocket_upgrade() {
        let mut req = WorkexRequest::get("wss://example.com/ws");
        req.headers.set("upgrade", "websocket");
        req.headers.set("connection", "upgrade");
        assert!(WorkexWebSocket::is_upgrade(&req));
    }

    #[test]
    fn non_websocket_request() {
        let req = WorkexRequest::get("https://example.com/api");
        assert!(!WorkexWebSocket::is_upgrade(&req));
    }

    #[test]
    fn accept_upgrade() {
        let mut req = WorkexRequest::get("wss://example.com/ws");
        req.headers.set("upgrade", "websocket");
        req.headers.set("connection", "upgrade");

        let result = WorkexWebSocket::accept(&req);
        assert!(result.is_some());
        let (_, resp) = result.unwrap();
        assert_eq!(resp.status, 101);
        assert_eq!(resp.headers.get("upgrade"), Some("websocket"));
    }

    #[tokio::test]
    async fn websocket_send_recv() {
        let pair = WebSocketPair::new();
        let (mut client, mut server) = (pair.client, pair.server);

        // Client sends to server
        client.send(WsMessage::Text("hello".into())).await.unwrap();
        let msg = server.recv().await.unwrap();
        assert_eq!(msg, WsMessage::Text("hello".into()));

        // Server sends to client
        server.send(WsMessage::Text("world".into())).await.unwrap();
        let msg = client.recv().await.unwrap();
        assert_eq!(msg, WsMessage::Text("world".into()));
    }

    #[tokio::test]
    async fn websocket_binary() {
        let pair = WebSocketPair::new();
        let (mut client, mut server) = (pair.client, pair.server);

        client.send(WsMessage::Binary(vec![1, 2, 3])).await.unwrap();
        let msg = server.recv().await.unwrap();
        assert_eq!(msg, WsMessage::Binary(vec![1, 2, 3]));
    }
}
