//! Length-prefixed framing shared by the IPC client and the daemon server.
//!
//! One message is a 4-byte little-endian payload length followed by exactly that many bytes of
//! JSON. A connection carries one request and one response; these helpers only encode and decode
//! the prefix and the payload, so each side keeps its own bounds, cancellation checks and error
//! text.

use std::io::{Read, Write};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Upper bound on one framed IPC message (request or response), excluding the 4-byte length
/// prefix. Both sides enforce it: the client rejects a larger response length before reading the
/// body, and the daemon replaces an oversized response with a small `Error` response so an
/// oversized state snapshot is an error the caller can report instead of a connection failure.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Bytes of the little-endian payload length that precedes every frame.
const LENGTH_PREFIX_BYTES: usize = 4;

/// Whether an announced payload length exceeds [`MAX_PAYLOAD_BYTES`].
pub fn exceeds_payload_cap(length: usize) -> bool {
    length > MAX_PAYLOAD_BYTES
}

fn length_prefix(payload_len: usize) -> [u8; LENGTH_PREFIX_BYTES] {
    (payload_len as u32).to_le_bytes()
}

fn payload_length(prefix: [u8; LENGTH_PREFIX_BYTES]) -> usize {
    u32::from_le_bytes(prefix) as usize
}

/// Writes the 4-byte little-endian length prefix of a frame carrying `payload_len` bytes.
pub fn write_length<W: Write>(writer: &mut W, payload_len: usize) -> std::io::Result<()> {
    writer.write_all(&length_prefix(payload_len))
}

/// Writes a frame payload after its length prefix.
pub fn write_payload<W: Write>(writer: &mut W, payload: &[u8]) -> std::io::Result<()> {
    writer.write_all(payload)
}

/// Reads a frame's 4-byte length prefix and returns the payload length it announces.
pub fn read_length<R: Read>(reader: &mut R) -> std::io::Result<usize> {
    let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
    reader.read_exact(&mut prefix)?;
    Ok(payload_length(prefix))
}

/// Reads a frame payload previously announced by [`read_length`] into `payload`.
pub fn read_payload<R: Read>(reader: &mut R, payload: &mut [u8]) -> std::io::Result<()> {
    reader.read_exact(payload)
}

/// Async [`write_length`] for the tokio server.
pub async fn write_length_async<W>(writer: &mut W, payload_len: usize) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(&length_prefix(payload_len)).await
}

/// Async [`write_payload`] for the tokio server.
pub async fn write_payload_async<W>(writer: &mut W, payload: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(payload).await
}

/// Async [`read_length`] for the tokio server.
pub async fn read_length_async<R>(reader: &mut R) -> std::io::Result<usize>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
    reader.read_exact(&mut prefix).await?;
    Ok(payload_length(prefix))
}

/// Async [`read_payload`] for the tokio server.
pub async fn read_payload_async<R>(reader: &mut R, payload: &mut [u8]) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
{
    // Tokio's `read_exact` reports the number of bytes read; a frame helper returns `()`.
    reader.read_exact(payload).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_length_prefix_is_four_byte_little_endian() {
        let mut wire = Vec::new();
        write_length(&mut wire, 0x0102_0304).expect("write the frame length");
        assert_eq!(wire, [0x04, 0x03, 0x02, 0x01]);

        let mut reader = wire.as_slice();
        assert_eq!(
            read_length(&mut reader).expect("read the frame length"),
            0x0102_0304
        );
    }

    #[test]
    fn test_sync_frame_round_trip() {
        let payload = br#"{"type":"Ping"}"#;
        let mut wire = Vec::new();
        write_length(&mut wire, payload.len()).expect("write the frame length");
        write_payload(&mut wire, payload).expect("write the frame payload");

        let mut reader = wire.as_slice();
        let length = read_length(&mut reader).expect("read the frame length");
        assert_eq!(length, payload.len());
        let mut decoded = vec![0u8; length];
        read_payload(&mut reader, &mut decoded).expect("read the frame payload");
        assert_eq!(decoded, payload.to_vec());
    }

    #[test]
    fn test_payload_cap_is_sixteen_mebibytes() {
        assert_eq!(MAX_PAYLOAD_BYTES, 16 * 1024 * 1024);
        assert!(!exceeds_payload_cap(MAX_PAYLOAD_BYTES));
        assert!(exceeds_payload_cap(MAX_PAYLOAD_BYTES + 1));
    }

    #[tokio::test]
    async fn test_async_frame_round_trip() {
        let payload = br#"{"type":"Pong"}"#;
        let (mut writer, mut reader) = tokio::io::duplex(64);
        write_length_async(&mut writer, payload.len())
            .await
            .expect("write the frame length");
        write_payload_async(&mut writer, payload)
            .await
            .expect("write the frame payload");

        let length = read_length_async(&mut reader)
            .await
            .expect("read the frame length");
        assert_eq!(length, payload.len());
        let mut decoded = vec![0u8; length];
        read_payload_async(&mut reader, &mut decoded)
            .await
            .expect("read the frame payload");
        assert_eq!(decoded, payload.to_vec());
    }
}
