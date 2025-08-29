#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::Duration;
use pcsc::{Context, Scope, ShareMode, Protocols, Error};
use slint::SharedString;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;

    // Clone a handle for UI updates
    let ui_handle = ui.as_weak();

    // Spawn background thread for NFC scanning
    thread::spawn(move || {
        // Initialize PC/SC context
        let ctx = match Context::establish(Scope::User) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to establish context: {}", e);
                return;
            }
        };

        // Get available readers
        let mut readers_buffer = [0; 2048];
        let readers = match ctx.list_readers(&mut readers_buffer) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to list readers: {}", e);
                return;
            }
        };

        let mut acr122u = None;
        for reader in readers {
            let reader_name = reader.to_string_lossy();
            if reader_name.contains("ACR122") {
                acr122u = Some(reader);
                break;
            }
        }

        let acr122u = match acr122u {
            Some(r) => r,
            None => {
                eprintln!("No ACR122U reader found!");
                return;
            }
        };

        let mut last_uid = String::new();

        loop {
            match ctx.connect(acr122u, ShareMode::Shared, Protocols::ANY) {
                Ok(card) => {
                    thread::sleep(Duration::from_millis(100));

                    let get_uid = [0xFF, 0xCA, 0x00, 0x00, 0x00];
                    let mut recv_buffer = [0; 256];

                    match card.transmit(&get_uid, &mut recv_buffer) {
                        Ok(response) => {
                            if response.len() >= 2
                                && response[response.len() - 2] == 0x90
                                && response[response.len() - 1] == 0x00
                            {
                                let uid = &response[..response.len() - 2];
                                let uid_str = uid.iter()
                                    .map(|b| format!("{:02X}", b))
                                    .collect::<Vec<String>>()
                                    .join("");

                                if uid_str != last_uid {
                                    last_uid = uid_str.clone();

                                    if let Some(ui) = ui_handle.upgrade() {
                                        // Update a label in UI
                                        ui.set_card_uid(SharedString::from(uid_str.clone()));
                                    }
                                }
                            }
                        }
                        Err(e) => eprintln!("Transmit error: {}", e),
                    }

                    let _ = card.disconnect(pcsc::Disposition::LeaveCard);
                    thread::sleep(Duration::from_millis(500));
                }
                Err(Error::NoSmartcard) => {
                    if !last_uid.is_empty() {
                        if let Some(ui) = ui_handle.upgrade() {
                            ui.set_card_uid(SharedString::from("Card removed"));
                        }
                        last_uid.clear();
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                Err(e) => {
                    eprintln!("Connect error: {}", e);
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });

    ui.run()?;
    Ok(())
}
