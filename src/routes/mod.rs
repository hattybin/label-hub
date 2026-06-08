pub mod d365;
pub mod inbound;
pub mod jobs;
pub mod preview;
pub mod printers;
pub mod receiving_labels;
pub mod settings;

use crate::state::{AppState, Printer};

/// Case-insensitive lookup of a printer profile by name.
pub fn find_printer(printers: &[Printer], name: &str) -> Option<Printer> {
    let target = name.trim().to_ascii_lowercase();
    printers
        .iter()
        .find(|p| p.name.to_ascii_lowercase() == target)
        .cloned()
}

/// Resolve a printer by name and send raw ZPL to it.
pub async fn send_to_printer(state: &AppState, printer_name: &str, zpl: &str) -> Result<(), String> {
    let printer = {
        let store = state.store.lock().await;
        find_printer(&store.printers, printer_name)
    };
    let printer = printer.ok_or_else(|| format!("unknown printer '{printer_name}'"))?;
    crate::printer::send_zpl(&printer.ip, printer.port, zpl).await
}
