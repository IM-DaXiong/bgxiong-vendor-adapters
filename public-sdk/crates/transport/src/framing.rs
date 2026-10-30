//! Async IO around the SINGLE native ports frame codec. There is no second DTO.
use bgx_adapter_v2_ports::{read_frame, resource_policy, wire::WorkerEnvelopeV2, write_frame};
use std::io::{self, Cursor};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<WorkerEnvelopeV2>> {
    // Reading is owned by one task for its entire lifetime: select cancellation
    // never discards a partially read frame and then resumes at the wrong byte.
    let mut header = [0u8; 4];
    if reader.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..]).await?;
    let size = u32::from_le_bytes(header) as usize;
    if size == 0
        || size as u64
            > resource_policy()["limits"]["maxFrameBytes"]
                .as_u64()
                .unwrap()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "worker frame exceeds public resource budget",
        ));
    }
    let mut bytes = Vec::with_capacity(size + header.len());
    bytes.extend_from_slice(&header);
    bytes.resize(size + header.len(), 0);
    reader.read_exact(&mut bytes[header.len()..]).await?;
    read_frame(&mut Cursor::new(bytes))
}
pub async fn write<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &WorkerEnvelopeV2,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, frame)?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}
