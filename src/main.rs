//! A bot that logs chat messages and the number that we've received to the console.

use std::sync::Arc;
use std::time::Duration;

use actix_web::{App, HttpServer, web};
use azalea::prelude::*;
use azalea::{
    ClientBuilder,
    app::AppExit,
    prelude::{Account, Component},
};
use tokio::sync::{Mutex, mpsc::Receiver};

use crate::buffer::FifoBuffer;
use crate::server::{MessageLog, bot_messages};
use crate::{
    bot::{BotLocation, Instruction, handle},
    server::{bot_chat, bot_get_slot, bot_jump, bot_slot, bot_sneak, bot_tab, bot_walk, index},
};

mod bot;
mod buffer;
mod message;
mod server;

pub const USERNAME: &str = "CoralGPTv0d1";
pub const PWD: &str = "REDACTED";
pub const PORT: u16 = 6767;

use tokio::signal;

#[derive(Default, Clone, Component)]
pub struct State {
    pub location: Arc<Mutex<BotLocation>>,
    pub going_to_bedwars: Arc<Mutex<bool>>,
    pub recv_instruction: Arc<Mutex<Option<Receiver<Instruction>>>>,
    pub conn: Arc<Mutex<Option<rusqlite::Connection>>>,
    pub messages: Arc<Mutex<FifoBuffer<1000, String>>>,
}

impl State {
    pub fn new(
        recv_instruction: Receiver<Instruction>,
        conn: rusqlite::Connection,
        messages: Arc<Mutex<FifoBuffer<1000, String>>>,
    ) -> Self {
        Self {
            conn: Arc::new(Mutex::new(Some(conn))),
            recv_instruction: Arc::new(Mutex::new(Some(recv_instruction))),
            messages,
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> AppExit {
    let (tx, rx) = tokio::sync::mpsc::channel::<Instruction>(10);
    let tx = Arc::new(tx);
    let connection = rusqlite::Connection::open("chat_log.db").expect("could not open db");
    let messages: Arc<Mutex<FifoBuffer<1000, String>>> = Arc::new(Mutex::new(FifoBuffer::new()));
    connection
        .execute(
            "CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        username TEXT NOT NULL,
        timestamp TEXT DEFAULT (datetime('now')),
        starcount INTEGER DEFAULT 0,
        rank TEXT NOT NULL,
        message TEXT NOT NULL,
        ping INTEGER
    );",
            [],
        )
        .expect("failed instantiating db");

    let messages_bot = messages.clone();
    let server = tokio::spawn(async move {
        eprintln!("listening on port {PORT}");
        eprintln!("UI available at http://127.0.0.1:{PORT}/");
        HttpServer::new(move || {
            let messages_clone = messages.clone();
            App::new()
                .app_data(web::Data::new(tx.clone()))
                .app_data(web::Data::new(MessageLog {
                    messages: messages_clone,
                }))
                .service(index)
                .service(bot_chat)
                .service(bot_tab)
                .service(bot_jump)
                .service(bot_walk)
                .service(bot_sneak)
                .service(bot_slot)
                .service(bot_get_slot)
                .service(bot_messages)
        })
        .bind(("127.0.0.1", PORT))
        .expect("server failed")
        .run()
        .await
    });

    let account = Account::offline(USERNAME);
    let state = State::new(rx, connection, messages_bot);

    let client = ClientBuilder::new()
        .set_handler(handle)
        .set_state(state)
        .reconnect_after(Duration::from_secs(60))
        .start(account, "wind.coralmc.it");

    tokio::select! {
        _ = shutdown_signal() => {
            eprintln!("shutting down...");
        }
        _ = client => {
            eprintln!("client exited...");
        }
    }

    server.abort();
    AppExit::Success
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = signal::ctrl_c() => {
            eprintln!("shutting down via Ctrl+C");
        }
        _ = sigterm.recv() => {
            eprintln!("shutting down via SIGTERM");
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    signal::ctrl_c().await.expect("failed to listen for Ctrl+C");
    eprintln!("shutting down");
}
