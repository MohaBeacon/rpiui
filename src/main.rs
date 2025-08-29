#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::Duration;
use pcsc::{Context, Scope, ShareMode, Protocols, Error};
use slint::SharedString;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;

    // Get a weak handle so background thread can update the UI
    let ui_handle = ui.as_weak();

    // Spawn NFC scanning thread
    thread::spawn(move || {
        // Establish PC/SC context
        let ctx = match Context::establish(Scope::User) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to establish PC/SC context: {}", e);
                return;
            }
        };

        // List readers
        let mut readers_buffer = [0; 2048];
        let readers = match ctx.list_readers(&mut readers_buffer) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to list readers: {}", e);
                return;
            }
        };

        // Pick ACR122U reader
        let mut acr122u = None;
        for reader in readers {
            if reader.to_string_lossy().contains("ACR122") {
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

        // Main loop
        loop {
            match ctx.connect(acr122u, ShareMode::Shared, Protocols::ANY) {
                Ok(card) => {
                    thread::sleep(Duration::from_millis(100));

                    // APDU: Get card UID
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
                                    .collect::<Vec<_>>()
                                    .join("");

                                if uid_str != last_uid {
                                    last_uid = uid_str.clone();

                                    // Update UI (must use invoke_on_ui_thread)
                                    if let Some(ui) = ui_handle.upgrade() {
                                        let uid_clone = uid_str.clone();
                                        slint::invoke_from_event_loop(move || {
                                            ui.set_card_uid(SharedString::from(uid_clone));
                                        }).unwrap();
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
                            slint::invoke_from_event_loop(move || {
                                ui.set_card_uid(SharedString::from("Card removed"));
                            }).unwrap();
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

    // Run the UI (blocking)
    ui.run()?;
    Ok(())
}
