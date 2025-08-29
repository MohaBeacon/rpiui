#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::thread;
use std::time::Duration;
use pcsc::{Context, Scope, ShareMode, Protocols, Error};
use slint::SharedString;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    // background reader thread
    thread::spawn(move || {
        let ctx = Context::establish(Scope::User).unwrap();
        let mut readers_buffer = [0; 2048];
        let readers = ctx.list_readers(&mut readers_buffer).unwrap();

        // pick first ACR122U reader
        let acr122u = readers.into_iter()
            .find(|r| r.to_string_lossy().contains("ACR122"))
            .unwrap();

        let mut last_uid = String::new();

        loop {
            match ctx.connect(acr122u, ShareMode::Shared, Protocols::ANY) {
                Ok(card) => {
                    thread::sleep(Duration::from_millis(100));
                    let get_uid = [0xFF, 0xCA, 0x00, 0x00, 0x00];
                    let mut recv_buffer = [0; 256];

                    if let Ok(response) = card.transmit(&get_uid, &mut recv_buffer) {
                        if response.len() >= 2
                            && response[response.len()-2] == 0x90
                            && response[response.len()-1] == 0x00
                        {
                            let uid = &response[..response.len()-2];
                            let uid_str = uid.iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join("");

                            if uid_str != last_uid {
                                last_uid = uid_str.clone();
                                if let Some(ui) = ui_handle.upgrade() {
                                    let msg = format!("Card UID: {}", uid_str);
                                    slint::invoke_from_event_loop(move || {
                                        ui.set_card_uid(SharedString::from(msg));
                                    }).unwrap();
                                }
                            }
                        }
                    }
                    let _ = card.disconnect(pcsc::Disposition::LeaveCard);
                }
                Err(Error::NoSmartcard) => {
                    if !last_uid.is_empty() {
                        last_uid.clear();
                        if let Some(ui) = ui_handle.upgrade() {
                            slint::invoke_from_event_loop(move || {
                                ui.set_card_uid(SharedString::from("Card removed"));
                            }).unwrap();
                        }
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                Err(_) => thread::sleep(Duration::from_millis(500)),
            }
        }
    });

    ui.run()?; // UI loop
    Ok(())
}
