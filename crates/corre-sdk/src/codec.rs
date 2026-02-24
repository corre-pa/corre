//! Newline-delimited JSON codec for the CCPP transport layer.

use crate::protocol::{MAX_MESSAGE_BYTES, Message, Notification, Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Reads and writes newline-delimited JSON messages over async streams.
pub struct MessageCodec<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> {
    reader: BufReader<R>,
    writer: W,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> MessageCodec<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader: BufReader::new(reader), writer }
    }

    /// Read the next JSON-RPC message from the stream.
    /// Returns `None` on EOF.
    pub async fn read_message(&mut self) -> anyhow::Result<Option<Message>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed.len() > MAX_MESSAGE_BYTES {
            anyhow::bail!("message exceeds {MAX_MESSAGE_BYTES} byte limit ({} bytes)", trimmed.len());
        }

        let msg: Message = serde_json::from_str(trimmed)?;
        Ok(Some(msg))
    }

    /// Write a request as a newline-terminated JSON line.
    pub async fn write_request(&mut self, req: &Request) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(req)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Write a response as a newline-terminated JSON line.
    pub async fn write_response(&mut self, resp: &Response) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(resp)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Write a notification as a newline-terminated JSON line.
    pub async fn write_notification(&mut self, notif: &Notification) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(notif)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Write any serializable value as a newline-terminated JSON line.
    pub async fn write_raw(&mut self, value: &serde_json::Value) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(value)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
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
}
