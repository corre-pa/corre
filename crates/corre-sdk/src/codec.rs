//! Newline-delimited JSON codec for the CCPP transport layer.
//!
//! The codec can be used as a combined [`MessageCodec`] or split into independent
//! [`CodecReader`] and [`CodecWriter`] halves via [`MessageCodec::split`]. The split
//! form is used by [`AppClient`](crate::client::AppClient) to allow a
//! background reader task to run concurrently with writes.

use crate::protocol::{MAX_MESSAGE_BYTES, Message, Notification, Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

// ── Reader half ──────────────────────────────────────────────────────────

/// Read half of the CCPP codec: reads newline-delimited JSON messages.
pub struct CodecReader<R: AsyncRead + Unpin> {
    reader: BufReader<R>,
}

impl<R: AsyncRead + Unpin> CodecReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader: BufReader::new(reader) }
    }

    /// Read the next JSON-RPC message from the stream.
    /// Skips blank lines; returns `None` only on true EOF (zero bytes read).
    pub async fn read_message(&mut self) -> anyhow::Result<Option<Message>> {
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None);
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.len() > MAX_MESSAGE_BYTES {
                anyhow::bail!("message exceeds {MAX_MESSAGE_BYTES} byte limit ({} bytes)", trimmed.len());
            }

            let msg: Message = serde_json::from_str(trimmed)?;
            return Ok(Some(msg));
        }
    }
}

// ── Writer half ──────────────────────────────────────────────────────────

/// Write half of the CCPP codec: writes newline-delimited JSON messages.
pub struct CodecWriter<W: AsyncWrite + Unpin> {
    writer: W,
}

impl<W: AsyncWrite + Unpin> CodecWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Write a request as a newline-terminated JSON line.
    pub async fn write_request(&mut self, req: &Request) -> anyhow::Result<()> {
        self.write_json(req).await
    }

    /// Write a response as a newline-terminated JSON line.
    pub async fn write_response(&mut self, resp: &Response) -> anyhow::Result<()> {
        self.write_json(resp).await
    }

    /// Write a notification as a newline-terminated JSON line.
    pub async fn write_notification(&mut self, notif: &Notification) -> anyhow::Result<()> {
        self.write_json(notif).await
    }

    async fn write_json(&mut self, value: &impl serde::Serialize) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(value)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

// ── Combined codec ───────────────────────────────────────────────────────

/// Combined read/write codec. Can be split into independent halves via [`split`](Self::split).
pub struct MessageCodec<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> {
    reader: CodecReader<R>,
    writer: CodecWriter<W>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> MessageCodec<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader: CodecReader::new(reader), writer: CodecWriter::new(writer) }
    }

    /// Split into independent reader and writer halves.
    pub fn split(self) -> (CodecReader<R>, CodecWriter<W>) {
        (self.reader, self.writer)
    }

    pub async fn read_message(&mut self) -> anyhow::Result<Option<Message>> {
        self.reader.read_message().await
    }

    pub async fn write_request(&mut self, req: &Request) -> anyhow::Result<()> {
        self.writer.write_request(req).await
    }

    pub async fn write_response(&mut self, resp: &Response) -> anyhow::Result<()> {
        self.writer.write_response(resp).await
    }

    pub async fn write_notification(&mut self, notif: &Notification) -> anyhow::Result<()> {
        self.writer.write_notification(notif).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_request() {
        let req = Request::new(42, "mcp/callTool", Some(serde_json::json!({"server_name": "brave-search"})));
        let mut buf = Vec::new();
        {
            let reader: &[u8] = &[];
            let mut codec = MessageCodec::new(reader, &mut buf);
            codec.write_request(&req).await.unwrap();
        }

        let reader = &buf[..];
        let writer = Vec::new();
        let mut codec = MessageCodec::new(reader, writer);
        let msg = codec.read_message().await.unwrap().unwrap();
        match msg {
            Message::Request(r) => {
                assert_eq!(r.id, 42);
                assert_eq!(r.method, "mcp/callTool");
            }
            _ => panic!("expected Request"),
        }
    }

    #[tokio::test]
    async fn eof_returns_none() {
        let reader: &[u8] = &[];
        let writer = Vec::new();
        let mut codec = MessageCodec::new(reader, writer);
        assert!(codec.read_message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn split_halves_work_independently() {
        let req = Request::new(1, "test", None);
        let (mut reader_half, mut writer_half) = MessageCodec::new(&b""[..], Vec::new()).split();

        // Writer half can write
        // (we can't easily test the output here since the writer owns the Vec,
        //  but we verify it doesn't panic)
        assert!(reader_half.read_message().await.unwrap().is_none());

        // Writer half writes to its own buffer
        writer_half.write_request(&req).await.unwrap();
    }
}
