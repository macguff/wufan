//! Transport helpers without a Tokio dependency; the TSF DLL never owns a runtime.
#[cfg(windows)]
pub mod client;
#[cfg(windows)]
pub mod security;
pub use ime_protocol::wire::{decode, encode, frame_len, MAX_FRAME};

/// Blocking diagnostics only. A TSF adapter must use overlapped I/O on a worker.
pub fn read_frame(reader: &mut impl std::io::Read) -> Result<Vec<u8>, String> {
    let mut header = [0; 4];
    reader.read_exact(&mut header).map_err(|e| e.to_string())?;
    let mut body = vec![0; frame_len(header)?];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}
