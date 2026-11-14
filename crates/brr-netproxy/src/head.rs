use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::HEAD_MAX_BYTES;

const HEAD_END: &[u8] = b"\r\n\r\n";

pub(crate) enum HeadError {
    TooLarge,
    Io(io::Error),
}

// Consumes exactly through the blank line: the body (or tunnel data) follows the head on the same
// stream, so whatever the reader buffered past it stays in the reader for the caller.
pub(crate) async fn read_head<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, HeadError> {
    let mut head = Vec::new();

    loop {
        let available = reader.fill_buf().await.map_err(HeadError::Io)?;
        if available.is_empty() {
            return Err(HeadError::Io(io::ErrorKind::UnexpectedEof.into()));
        }

        let mut used = 0;
        for &byte in available {
            head.push(byte);
            used += 1;

            if head.ends_with(HEAD_END) {
                reader.consume(used);

                return Ok(head);
            }
            if head.len() == HEAD_MAX_BYTES {
                return Err(HeadError::TooLarge);
            }
        }
        reader.consume(used);
    }
}

pub(crate) fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
