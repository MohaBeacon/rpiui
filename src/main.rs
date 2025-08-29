#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::Duration;
use pcsc::{Context, Scope, ShareMode, Protocols, Error};
use slint::SharedString;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;

    // Create a Weak handle for thread-safe access
    let ui_handle = ui.as_weak();

    // Spawn NFC scanning in background
    thread::spawn(move || {
        let ctx = match Context::establish(Scope::User) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to establish PC/SC context: {}", e);
                return;
            }
        };

        // List available readers
        let mut readers_buffer = [0; 2048];
        let readers = match ctx.list_readers(&mut readers_buffer) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to list readers: {}", e);
                return;
            }
        };

        // Find ACR122U
        let acr122u = match readers.into_iter()
            .find(|r| r.to_string_lossy().contains("ACR122"))
        {
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

                    if let Ok(response) = card.transmit(&get_uid, &mut recv_buffer) {
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

                                let weak = ui_handle.clone();
                                let msg = format!("Card UID: {}", uid_str);

                                slint::invoke_from_event_loop(move || {
                                    if let Some(ui) = weak.upgrade() {
                                        ui.set_card_uid(SharedString::from(msg));
                                    }
                                }).unwrap();
                            }
                        }
                    }

                    let _ = card.disconnect(pcsc::Disposition::LeaveCard);
                    thread::sleep(Duration::from_millis(500));
                }
                Err(Error::NoSmartcard) => {
                    if !last_uid.is_empty() {
                        last_uid.clear();

                        let weak = ui_handle.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_card_uid(SharedString::from("Card removed"));
                            }
                        }).unwrap();
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

    // Run the UI loop (blocking)
    ui.run()?;
    Ok(())
}
