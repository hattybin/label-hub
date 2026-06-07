//! Optional mDNS / DNS-SD advertisement so the local console is reachable by name
//! (e.g. `http://printlabels.local:8081`) without users memorising IPs or the
//! tunnel host. Uses the pure-Rust `mdns-sd` responder — no Avahi dependency.
//!
//! Returns the running daemon; keep it alive for the process lifetime (dropping it
//! unregisters the service).

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// Advertise the console as `_http._tcp.local` and publish an A record for
/// `<hostname>.local` on all active interfaces.
pub fn advertise(hostname_fqdn: &str, port: u16, site: &str) -> Result<ServiceDaemon, String> {
    let daemon = ServiceDaemon::new().map_err(|e| format!("mDNS daemon failed to start: {e}"))?;

    // mdns-sd expects a trailing dot on the host name.
    let host = format!("{}.", hostname_fqdn.trim_end_matches('.'));
    let instance = format!("Label Hub ({site})");

    // Empty IP + enable_addr_auto() → the responder fills in (and keeps updated)
    // every active interface address automatically.
    let info = ServiceInfo::new(
        "_http._tcp.local.",
        &instance,
        &host,
        "",
        port,
        &[("path", "/")][..],
    )
    .map_err(|e| format!("invalid mDNS service info: {e}"))?
    .enable_addr_auto();

    daemon
        .register(info)
        .map_err(|e| format!("mDNS register failed: {e}"))?;

    Ok(daemon)
}
