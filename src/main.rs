//! A bot that logs chat messages and the number that we've received to the console.

use std::sync::Arc;

use actix_web::{App, HttpServer, web};
use azalea::prelude::*;
use azalea::{
    ClientBuilder,
    app::AppExit,
    prelude::{Account, Component},
};
use tokio::sync::{Mutex, mpsc::Receiver};

use crate::{
    bot::{BotLocation, Instruction, handle},
    server::{bot_chat, bot_jump, bot_tab},
};

mod bot;
mod message;
mod server;

pub const USERNAME: &str = "CoralGPTv0d1";
pub const PWD: &str = "REDACTED";

use tokio::signal;

#[derive(Default, Clone, Component)]
pub struct State {
    pub location: Arc<Mutex<BotLocation>>,
    pub going_to_bedwars: Arc<Mutex<bool>>,
    pub recv_instruction: Arc<Mutex<Option<Receiver<Instruction>>>>,
    pub conn: Arc<Mutex<Option<rusqlite::Connection>>>,
}

impl State {
    pub fn new(recv_instruction: Receiver<Instruction>, conn: rusqlite::Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(Some(conn))),
            recv_instruction: Arc::new(Mutex::new(Some(recv_instruction))),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> AppExit {
    let (tx, rx) = tokio::sync::mpsc::channel::<Instruction>(10);
    let tx = Arc::new(tx);
    let connection = rusqlite::Connection::open("chat_log.db").expect("could not open db");
    connection
        .execute(
            "CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        username TEXT NOT NULL,
        timestamp TEXT DEFAULT (datetime('now')),
        starcount INTEGER DEFAULT 0,
        rank TEXT NOT NULL,
        message TEXT NOT NULL
    );",
            [],
        )
        .expect("failed instantiating db");

    let server = tokio::spawn(async move {
        eprintln!("listening on port 8080");
        eprintln!("available routes: Post(chat), Post(jump), Post(tab)");
        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(tx.clone()))
                .service(bot_chat)
                .service(bot_tab)
                .service(bot_jump)
        })
        .bind(("127.0.0.1", 8080))
        .expect("server failed")
        .run()
        .await
    });

    let account = Account::offline(USERNAME);
    let state = State::new(rx, connection);

    let client = ClientBuilder::new()
        .set_handler(handle)
        .set_state(state)
        .start(account, "wind.coralmc.it");

    tokio::select! {
        _ = signal::ctrl_c() => {
            eprintln!("shutting down...");
        }
        _ = client => {
            eprintln!("client exited");
        }
    }

    server.abort();
    AppExit::Success
}
