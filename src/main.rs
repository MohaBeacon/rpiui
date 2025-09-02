#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::Duration;
use pcsc::{Context, Scope, ShareMode, Protocols, Error};
use slint::SharedString;
use lazy_static::lazy_static;

slint::include_modules!();

// Configuration struct
struct Config {
    scan_interval: Duration,
    stabilize_delay: Duration,
    reader_name: String,
    valid_uid_lengths: Vec<usize>,
}

lazy_static! {
    static ref CONFIG: Config = Config {
        scan_interval: Duration::from_millis(200),
        stabilize_delay: Duration::from_millis(100),
        reader_name: "ACR122".to_string(),
        valid_uid_lengths: vec![4, 7, 10], // Common NFC UID lengths
    };
}

// Helper function to show errors in UI
fn show_error(ui_handle: &slint::Weak<AppWindow>, message: &str) {
    let weak = ui_handle.clone();
    let msg = message.to_string();
    slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_card_uid(SharedString::from(format!("Error: {}", msg)));
        }
    }).unwrap();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize Slint UI
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    // Spawn NFC scanning thread
    thread::spawn(move || {
        // Establish PC/SC context
        let ctx = match Context::establish(Scope::User) {
            Ok(c) => c,
            Err(e) => {
                show_error(&ui_handle, &format!("Failed to establish PC/SC context: {}", e));
                return;
            }
        };

        // List available readers
        let mut readers_buffer = [0; 2048];
        let readers = match ctx.list_readers(&mut readers_buffer) {
            Ok(r) => r,
            Err(e) => {
                show_error(&ui_handle, &format!("Failed to list readers: {}", e));
                return;
            }
        };

        // Find ACR122U
        let acr122u = match readers.into_iter()
            .find(|r| r.to_string_lossy().contains(&CONFIG.reader_name))
        {
            Some(r) => r,
            None => {
                show_error(&ui_handle, "No ACR122U reader found!");
                return;
            }
        };

        let mut last_uid = String::new();

        // Main scanning loop
        loop {
            match ctx.connect(acr122u, ShareMode::Shared, Protocols::ANY) {
                Ok(card) => {
                    thread::sleep(CONFIG.stabilize_delay);

                    let get_uid = [0xFF, 0xCA, 0x00, 0x00, 0x00];
                    let mut recv_buffer = [0; 256];

                    if let Ok(response) = card.transmit(&get_uid, &mut recv_buffer) {
                        if response.len() >= 2
                            && response[response.len() - 2] == 0x90
                            && response[response.len() - 1] == 0x00
                        {
                            let uid = &response[..response.len() - 2];
                            if CONFIG.valid_uid_lengths.contains(&uid.len()) {
                                let uid_str = uid
                                    .iter()
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
                                            ui.set_current_screen(SharedString::from("welcome")); // Move to welcome screen
                                            // Optionally set user-name if you have a way to map UID to a name
                                            // ui.set_user_name(SharedString::from("User"));
                                        }
                                    }).unwrap();
                                }
                            } else {
                                show_error(&ui_handle, &format!("Invalid UID length: {}", uid.len()));
                            }
                        } else {
                            show_error(
                                &ui_handle,
                                &format!(
                                    "Invalid response: {:02X} {:02X}",
                                    response[response.len() - 2],
                                    response[response.len() - 1]
                                ),
                            );
                        }
                    } else {
                        show_error(&ui_handle, "Failed to read card");
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
                                ui.set_card_uid(SharedString::from("Waiting for card..."));
                                // Return to preintro when card is removed
                            }
                        }).unwrap();
                    }
                    thread::sleep(CONFIG.scan_interval);
                }
                Err(e) => {
                    show_error(&ui_handle, &format!("Connect error: {}", e));
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });

    // Run the UI loop
    ui.run()?;
    Ok(())
}