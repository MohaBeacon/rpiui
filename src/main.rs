#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use std::thread;
use std::time::Duration;
use pcsc::{Context, Scope, ShareMode, Protocols, Error};
use slint::{SharedString, Weak};
slint::include_modules!();

// Configuration struct for NFC
struct Config {
    scan_interval: Duration,
    stabilize_delay: Duration,
    reader_name: String,
    valid_uid_lengths: Vec<usize>,
}

lazy_static::lazy_static! {
    static ref CONFIG: Config = Config {
        scan_interval: Duration::from_millis(200),
        stabilize_delay: Duration::from_millis(100),
        reader_name: "ACR122".to_string(),
        valid_uid_lengths: vec![4, 7, 10],
    };
}

// Define error types for API
#[derive(Error, Debug)]
enum AppError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Event ID not found in response")]
    MissingEventId,
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("PCSC error: {0}")]
    Pcsc(#[from] pcsc::Error),
    #[error("Event loop error: {0}")]
    EventLoop(#[from] slint::EventLoopError),
}

// Define the POST request payload for the get_by_slug endpoint
#[derive(Serialize)]
struct PostPayload {
    access_token: String,
    slug: String,
}

// Define the POST request payload for the guests endpoint
#[derive(Serialize)]
struct GuestsPostPayload {
    access_token: String,
    guest_tag: String,
}

// Define the POST request payload for the load_score endpoint
#[derive(Serialize, Debug)]
struct LoadScorePostPayload {
    access_token: String,
    checkpoint_id: i32,
    guest_tag: String,
    score: String,
}

// Define the expected POST response structure for the get_by_slug endpoint
#[derive(Deserialize, Serialize, Debug)]
struct Checkpoint {
    event_id: i32,
    id: i32,
    name: String,
    repetible: i32,
    score: i32,
    slug: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct PostResponse {
    checkpoint: Checkpoint,
}

// Define the expected POST response structure for the guests endpoint
#[derive(Deserialize, Serialize, Debug)]
struct Guest {
    name: String,
    tag: Option<String>, // Explicitly map the tag field
    #[serde(flatten)]
    other: serde_json::Value,
}

#[derive(Deserialize, Serialize, Debug)]
struct GuestsPostResponse {
    guests: Vec<Guest>,
}

// Alternative struct if the response is nested (e.g., {"data": {"guests": [...]}})
/*
#[derive(Deserialize, Serialize, Debug)]
struct GuestsPostResponse {
    data: GuestsData,
}

#[derive(Deserialize, Serialize, Debug)]
struct GuestsData {
    guests: Vec<Guest>,
}
*/

// Define the expected POST response structure for the load_score endpoint
#[derive(Deserialize, Serialize, Debug)]
struct LoadScorePostResponse {
    #[serde(flatten)]
    data: serde_json::Value,
}

// Function to validate inputs
fn validate_inputs(access_token: &str, slug: &str, guest_tags: &[String], score: &str) -> Result<(), AppError> {
    if access_token.is_empty() {
        return Err(AppError::InvalidInput("Access token cannot be empty".to_string()));
    }
    if slug.is_empty() {
        return Err(AppError::InvalidInput("Slug cannot be empty".to_string()));
    }
    if guest_tags.is_empty() {
        return Err(AppError::InvalidInput("At least one guest tag is required".to_string()));
    }
    for tag in guest_tags {
        if tag.is_empty() {
            return Err(AppError::InvalidInput("Guest tags cannot be empty".to_string()));
        }
    }
    if score.is_empty() {
        return Err(AppError::InvalidInput("Score cannot be empty".to_string()));
    }
    if score.parse::<i32>().is_err() {
        return Err(AppError::InvalidInput("Score must be a valid integer".to_string()));
    }
    Ok(())
}

// Function for the get_by_slug POST request with retry logic
fn post_get_by_slug(
    client: &Client,
    access_token: &str,
    slug: &str,
    max_retries: u32,
) -> Result<PostResponse, AppError> {
    let post_url = "https://wonderlab.events/controlacceso/v2/api/checkpoints/get_by_slug";
    let payload = PostPayload {
        access_token: access_token.to_string(),
        slug: slug.to_string(),
    };

    for attempt in 1..=max_retries {
        let response = client
            .post(post_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send();

        match response {
            Ok(resp) => match resp.status() {
                reqwest::StatusCode::OK => {
                    let json_response = resp.json::<PostResponse>()?;
                    println!(
                        "post_get_by_slug response: {}",
                        serde_json::to_string_pretty(&json_response).unwrap_or_else(|_| "Failed to serialize response".to_string())
                    );
                    return Ok(json_response);
                }
                status @ (reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE) => {
                    if attempt == max_retries {
                        let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                        return Err(AppError::ApiError {
                            status: status.as_u16(),
                            message,
                        });
                    }
                    thread::sleep(Duration::from_secs(1 << attempt));
                }
                status => {
                    let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(AppError::ApiError {
                        status: status.as_u16(),
                        message,
                    });
                }
            },
            Err(e) => {
                if attempt == max_retries {
                    return Err(AppError::from(e));
                }
                thread::sleep(Duration::from_secs(1 << attempt));
            }
        }
    }
    Err(AppError::ApiError {
        status: 0,
        message: "Max retries reached".to_string(),
    })
}

// Function for the visual GET request with retry logic
fn get_visual(
    client: &Client,
    access_token: &str,
    event_id: i32,
    max_retries: u32,
) -> Result<serde_json::Value, AppError> {
    let get_url = format!(
        "https://wonderlab.events/controlacceso/v2/api/checkpoints/visual/{}",
        event_id
    );

    for attempt in 1..=max_retries {
        let response = client
            .get(&get_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send();

        match response {
            Ok(resp) => match resp.status() {
                reqwest::StatusCode::OK => {
                    let json_response = resp.json::<serde_json::Value>()?;
                    println!(
                        "get_visual response: {}",
                        serde_json::to_string_pretty(&json_response).unwrap_or_else(|_| "Failed to serialize response".to_string())
                    );
                    return Ok(json_response);
                }
                status @ (reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE) => {
                    if attempt == max_retries {
                        let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                        return Err(AppError::ApiError {
                            status: status.as_u16(),
                            message,
                        });
                    }
                    thread::sleep(Duration::from_secs(1 << attempt));
                }
                status => {
                    let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(AppError::ApiError {
                        status: status.as_u16(),
                        message,
                    });
                }
            },
            Err(e) => {
                if attempt == max_retries {
                    return Err(AppError::from(e));
                }
                thread::sleep(Duration::from_secs(1 << attempt));
            }
        }
    }
    Err(AppError::ApiError {
        status: 0,
        message: "Max retries reached".to_string(),
    })
}

// Function for the guests POST request with retry logic
fn post_guests(
    client: &Client,
    access_token: &str,
    guest_tag: &str,
    max_retries: u32,
) -> Result<GuestsPostResponse, AppError> {
    let post_url = "https://wonderlab.events/controlacceso/v2/api/control/guests";
    let payload = GuestsPostPayload {
        access_token: access_token.to_string(),
        guest_tag: guest_tag.to_string(),
    };

    for attempt in 1..=max_retries {
        let response = client
            .post(post_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&payload)
            .send();

        match response {
            Ok(resp) => match resp.status() {
                reqwest::StatusCode::OK => {
                    let text = resp.text()?;
                    println!("Raw JSON response: {}", text);
                    let json_response = serde_json::from_str::<GuestsPostResponse>(&text).map_err(|e| {
                        println!("Deserialization error: {}", e);
                        AppError::Json(e)
                    })?;
                    println!("Deserialized response: {:?}", json_response);
                    return Ok(json_response);
                }
                status @ (reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE) => {
                    if attempt == max_retries {
                        let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                        return Err(AppError::ApiError {
                            status: status.as_u16(),
                            message,
                        });
                    }
                    thread::sleep(Duration::from_secs(1 << attempt));
                }
                status => {
                    let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(AppError::ApiError {
                        status: status.as_u16(),
                        message,
                    });
                }
            },
            Err(e) => {
                if attempt == max_retries {
                    return Err(AppError::from(e));
                }
                thread::sleep(Duration::from_secs(1 << attempt));
            }
        }
    }
    Err(AppError::ApiError {
        status: 0,
        message: "Max retries reached".to_string(),
    })
}

// Function for the load_score POST request with retry logic
fn post_load_score(
    client: &Client,
    access_token: &str,
    checkpoint_id: i32,
    guest_tag: &str,
    score: &str,
    max_retries: u32,
) -> Result<LoadScorePostResponse, AppError> {
    let post_url = "https://wonderlab.events/controlacceso/v2/api/checkpoints/load_score";
    let payload = LoadScorePostPayload {
        access_token: access_token.to_string(),
        checkpoint_id,
        guest_tag: guest_tag.to_string(),
        score: score.to_string(),
    };

    for attempt in 1..=max_retries {
        let response = client
            .post(post_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&payload)
            .send();

        match response {
            Ok(resp) => match resp.status() {
                reqwest::StatusCode::OK => {
                    let json_response = resp.json::<LoadScorePostResponse>()?;
                    println!(
                        "post_load_score response: {}",
                        serde_json::to_string_pretty(&json_response).unwrap_or_else(|_| "Failed to serialize response".to_string())
                    );
                    return Ok(json_response);
                }
                reqwest::StatusCode::CONFLICT => {
                    let json_response = LoadScorePostResponse {
                        data: serde_json::json!({ "message": "Score already loaded" }),
                    };
                    println!(
                        "post_load_score response (CONFLICT): {}",
                        serde_json::to_string_pretty(&json_response).unwrap_or_else(|_| "Failed to serialize response".to_string())
                    );
                    return Ok(json_response);
                }
                status @ (reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE) => {
                    if attempt == max_retries {
                        let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                        return Err(AppError::ApiError {
                            status: status.as_u16(),
                            message,
                        });
                    }
                    thread::sleep(Duration::from_secs(1 << attempt));
                }
                status => {
                    let message = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(AppError::ApiError {
                        status: status.as_u16(),
                        message,
                    });
                }
            },
            Err(e) => {
                if attempt == max_retries {
                    return Err(AppError::from(e));
                }
                thread::sleep(Duration::from_secs(1 << attempt));
            }
        }
    }
    Err(AppError::ApiError {
        status: 0,
        message: "Max retries reached".to_string(),
    })
}

// Function to handle multiple guest tags for guests and load_score
fn post_multiple_guests_and_scores(
    client: &Client,
    access_token: &str,
    guest_tags: &[String],
    checkpoint_id: &str,
    score: &str,
    max_retries: u32,
    ui_handle: Weak<AppWindow>,
) -> Result<(Vec<GuestsPostResponse>, Vec<LoadScorePostResponse>), AppError> {
    let mut guests_responses = Vec::new();
    let mut load_score_responses = Vec::new();

    let checkpoint_id: i32 = checkpoint_id.parse().map_err(|_| {
        AppError::InvalidInput("Checkpoint ID must be a valid integer".to_string())
    })?;

    for guest_tag in guest_tags {
        let guests_response = post_guests(client, access_token, guest_tag, max_retries)?;
        let username = guests_response.guests.get(0).map(|g| g.name.clone()).unwrap_or_default();
        if username.is_empty() {
            let weak = ui_handle.clone();
            let username = SharedString::from(username);
            slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                }
            }).unwrap_or_else(|e| eprintln!("Event loop error: {}", e));
        }
        guests_responses.push(guests_response);

        let load_score_response = post_load_score(
            client,
            access_token,
            checkpoint_id,
            guest_tag,
            score,
            max_retries,
        )?;
        load_score_responses.push(load_score_response);
    }

    Ok((guests_responses, load_score_responses))
}

// Helper function to show errors in UI
fn show_error(ui_handle: &Weak<AppWindow>, message: &str) {
    let weak = ui_handle.clone();
    let msg = message.to_string();
    slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_card_uid(SharedString::from(format!("Error: {}", msg)));
        }
    }).unwrap_or_else(|e| eprintln!("Event loop error: {}", e));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize Slint UI
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    // API configuration
    let access_token = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOjMwLCJyb2xlIjoiY29udHJvbCJ9.OjbB_aLB6KnBXEeMpKP9HZMMN73zm_-0mBuvNyDvSpI".to_string();
    let slug ="checkpoint-prueba-546".to_string();

    // Initialize HTTP client
    let client = Client::new();

    // Set up UI callback to handle score submission
    ui.on_submit_score({
        let access_token = access_token.clone();
        let slug = slug.clone();
        let client = client.clone();
        let ui_handle_clone = ui_handle.clone();

        move |score: SharedString| {
            println!("Score to submit: {}", score);
            let ui_handle = ui_handle_clone.clone();
            let access_token = access_token.clone();
            let slug = slug.clone();
            let score = score.to_string();
            let client = client.clone();

            // Step 1: Fetch post_get_by_slug response
            let post_response = match post_get_by_slug(&client, &access_token, &slug, 3) {
                Ok(resp) => resp,
                Err(e) => {
                    show_error(&ui_handle, &format!("Failed to fetch checkpoint: {}", e));
                    return;
                }
            };
            let trivia_name = if let Some(ui) = ui_handle.upgrade() {
                let name = ui.get_trivia_name().to_string();
                name
            } else {
                String::new()
            };
            println!("Retrieved trivia_name: {}", trivia_name);
            let valueoftrivia = trivia_name.clone();
            let mut gettag = if let Some(ui) = ui_handle.upgrade() {
                let tag = ui.get_card_uid().to_string();
                tag
            } else {
                String::new()
            };
            println!("Retrieved gettag: {}", gettag);

            // Step 3: Map trivia_name to checkpoint_id
            let checkpoint_id = match valueoftrivia.as_str() {
                "TRIVIA 1" => "62",
                "TRIVIA 2" => "63",
                _ => {
                    show_error(&ui_handle, "Invalid trivia name");
                    return;
                }
            };
            let checkpoint_id: i32 = match checkpoint_id.parse() {
                Ok(id) => id,
                Err(_) => {
                    show_error(&ui_handle, "Checkpoint ID must be a valid integer");
                    return;
                }
            };
            println!("Mapped checkpoint_id: {}", checkpoint_id);

            let guest_tags = &gettag;

            let score_response = match post_load_score(
                &client,
                &access_token,
                checkpoint_id,
                &guest_tags,
                &score,
                3,
            ) {
                Ok(resp) => {
                    println!("post_load_score response: {:?}", resp);
                    resp
                }
                Err(e) => {
                    println!("post_load_score error: {:?}", e);
                    show_error(&ui_handle, &format!("Failed to load score: {:?}", e));
                    return;
                }
            };
            println!("post_load_score completed: {:?}", score_response);
        }
    });

    // Spawn NFC scanning thread
    thread::spawn(move || {
        let ctx = match Context::establish(Scope::User) {
            Ok(c) => c,
            Err(e) => {
                show_error(&ui_handle, &format!("Failed to establish PC/SC context: {}", e));
                return;
            }
        };

        let mut readers_buffer = [0; 2048];
        let readers = match ctx.list_readers(&mut readers_buffer) {
            Ok(r) => r,
            Err(e) => {
                show_error(&ui_handle, &format!("Failed to list readers: {}", e));
                return;
            }
        };

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
                                    let response = match post_guests(&client, &access_token, &uid_str, 3) {
                                        Ok(resp) => resp,
                                        Err(e) => {
                                            show_error(&ui_handle, &format!("Failed to fetch guests: {}", e));
                                            return;
                                        }
                                    };

                                    println!("Guests response: {:?}", response);

                                    let mut username = String::new();
                                    let mut tag = String::new();

                                    if let Some(guest) = response.guests.get(0) {
                                        username = guest.name.clone();
                                        tag = guest.tag.clone().unwrap_or_default();
                                        if tag.is_empty() {
                                            show_error(&ui_handle, "Guest tag is missing in response");
                                        } else {
                                            println!("Guest: {}, Tag: {}", username, tag);
                                        }
                                    } else {
                                        show_error(&ui_handle, "No guests found in response");
                                    }

                                    let weak = ui_handle.clone();
                                    slint::invoke_from_event_loop(move || {
                                        if let Some(ui) = weak.upgrade() {
                                            ui.set_user_name(SharedString::from(username));
                                            ui.set_current_screen(SharedString::from("welcome"));
                                            ui.set_card_uid(SharedString::from(tag));
                                        }
                                    }).unwrap_or_else(|e| eprintln!("Event loop error: {}", e));
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
                            }
                        }).unwrap_or_else(|e| eprintln!("Event loop error: {}", e));
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