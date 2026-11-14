use std::io;

use axum::body::Bytes;
use brr_core::Error;
use futures::{Stream, stream};
use tokio::{fs::File, io::AsyncReadExt};

pub const CLIENT_HEADER: &str = "Brr-Client";
pub const CLIENT_KIND_HEADER: &str = "Brr-Client-Kind";
pub const CLI_KIND: &str = "cli";
pub const PASSPHRASE_HEADER: &str = "X-Brr-Passphrase";
pub const ARCHIVE_MIME: &str = "application/octet-stream";
pub const LAGGED: &str = "lagged";
pub const NORMAL_CLOSE: u16 = 1000;
pub const BYTES_PER_MIB: u64 = 1024 * 1024;
// What a JSON request or reply carries as a file; bigger ones go through the raw streaming routes.
pub const MAX_INLINE_FILE_BYTES: u64 = 48 * BYTES_PER_MIB;

const READ_CHUNK: usize = 64 * 1024;

pub fn websocket_only() -> Error {
    Error::InvalidArgument {
        field: "upgrade".to_string(),
        message: "this route speaks WebSocket; connect with a WebSocket client".to_string(),
    }
}

// `keep` rides along in the stream state, so whatever backs the file (a scratch dir) lives until
// the body ends or is dropped.
pub fn file_chunks<K>(file: File, keep: K) -> impl Stream<Item = io::Result<Bytes>> {
    stream::unfold((file, keep), |(mut file, keep)| async move {
        let mut chunk = vec![0; READ_CHUNK];
        match file.read(&mut chunk).await {
            Ok(0) => None,
            Ok(read) => {
                chunk.truncate(read);

                Some((Ok(Bytes::from(chunk)), (file, keep)))
            }
            Err(error) => Some((Err(error), (file, keep))),
        }
    })
}
