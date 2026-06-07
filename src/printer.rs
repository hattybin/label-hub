//! Raw ZPL delivery over a TCP socket (the standard Zebra "RAW" path on port 9100)
//! and a lightweight reachability probe. Ported from the Node prototype's
//! `sendZplToPrinter` / `testPrinterConnection`.

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

/// Open a TCP connection to `ip:port` and write the raw ZPL bytes.
pub async fn send_zpl(ip: &str, port: u16, zpl: &str) -> Result<(), String> {
    let addr = format!("{ip}:{port}");
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| format!("timed out connecting to {addr}"))?
        .map_err(|e| format!("could not connect to {addr}: {e}"))?;

    timeout(WRITE_TIMEOUT, stream.write_all(zpl.as_bytes()))
        .await
        .map_err(|_| format!("timed out writing to {addr}"))?
        .map_err(|e| format!("write failed to {addr}: {e}"))?;

    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
    Ok(())
}

/// Returns true if a TCP connection to `ip:port` can be opened.
pub async fn is_reachable(ip: &str, port: u16) -> bool {
    let addr = format!("{ip}:{port}");
    matches!(
        timeout(Duration::from_secs(3), TcpStream::connect(&addr)).await,
        Ok(Ok(_))
    )
}

/// Strip ZPL control characters that would let label data break out of fields.
/// (Inbound jobs from D365 are already rendered ZPL, so this is only applied to
/// user-supplied field values, never to whole-payload ZPL.)
#[allow(dead_code)]
pub fn sanitize_field(s: &str) -> String {
    s.replace('^', "").replace('~', "")
}
