//! Streaming responses — chunked transfer for large bodies.
//! Workers API: `return new Response(readableStream)`

use bytes::Bytes;

use crate::headers::Headers;

/// A response that can stream its body in chunks.
pub struct StreamingResponse {
    pub status: u16,
    pub headers: Headers,
    pub body: StreamBody,
}

/// Response body — either buffered or streaming.
pub enum StreamBody {
    Buffer(Bytes),
    Chunks(Vec<Bytes>),
}

impl StreamingResponse {
    /// Create from a complete buffer.
    pub fn buffered(body: impl Into<Bytes>, status: u16) -> Self {
        Self {
            status,
            headers: Headers::new(),
            body: StreamBody::Buffer(body.into()),
        }
    }

    /// Create from chunked data.
    pub fn chunked(chunks: Vec<Bytes>, status: u16) -> Self {
        Self {
            status,
            headers: Headers::new(),
            body: StreamBody::Chunks(chunks),
        }
    }

    /// Collect all chunks into a single buffer.
    pub fn collect_body(&self) -> Bytes {
        match &self.body {
            StreamBody::Buffer(b) => b.clone(),
            StreamBody::Chunks(chunks) => {
                let total: usize = chunks.iter().map(|c| c.len()).sum();
                let mut buf = Vec::with_capacity(total);
                for chunk in chunks {
                    buf.extend_from_slice(chunk);
                }
                Bytes::from(buf)
            }
        }
    }

    /// Number of chunks (1 for buffered).
    pub fn chunk_count(&self) -> usize {
        match &self.body {
            StreamBody::Buffer(_) => 1,
            StreamBody::Chunks(c) => c.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_response() {
        let resp = StreamingResponse::buffered("hello", 200);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.collect_body(), Bytes::from("hello"));
        assert_eq!(resp.chunk_count(), 1);
    }

    #[test]
    fn chunked_response() {
        let chunks = vec![
            Bytes::from("chunk1"),
            Bytes::from("chunk2"),
            Bytes::from("chunk3"),
        ];
        let resp = StreamingResponse::chunked(chunks, 200);
        assert_eq!(resp.chunk_count(), 3);
        assert_eq!(resp.collect_body(), Bytes::from("chunk1chunk2chunk3"));
    }

    #[test]
    fn empty_chunks() {
        let resp = StreamingResponse::chunked(Vec::new(), 204);
        assert_eq!(resp.chunk_count(), 0);
        assert_eq!(resp.collect_body(), Bytes::new());
    }
}
