#![warn(clippy::pedantic, clippy::nursery)]
//! A bot that logs chat messages and the number that we've received to the console.

use actix_web::{App, HttpServer, web};
use azalea::{
    ClientBuilder,
    app::AppExit,
    prelude::{Account, Component},
};
use azalea::{FormattedText, prelude::*};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc::Receiver};
use tokio::{signal, task::JoinHandle};

use crate::server::{Broadcaster, MessageLog, bot_messages, ws_handler};
use crate::{
    bot::{BotLocation, Instruction, handle},
    server::{
        bot_agent, bot_chat, bot_get_slot, bot_jump, bot_slot, bot_sneak, bot_tab, bot_walk, index,
    },
};
use crate::{buffer::FifoBuffer, server::bot_get_stats};
use openai_client::OpenAIClient;

mod bot;
mod buffer;
mod message;
mod server;

pub struct EnvVars {
    pub username: String,
    pub password: String,
    pub port: u16,
    pub db_path: String,
    pub openai_client: Option<OpenAIClient>,
    pub bot_clan_name: Option<String>,
    pub discord_webhook_url: String,
}

pub static ENV_VARS: LazyLock<EnvVars> = LazyLock::new(|| {
    let username: String = dotenvy::var("USERNAME").expect("Please set USERNAME env var");
    let password: String = dotenvy::var("PASSWORD").expect("Please set PASSWORD env var");
    let bot_clan_name: Option<String> = dotenvy::var("BOT_CLAN_NAME").ok();
    let port: u16 = dotenvy::var("PORT")
        .expect("Please set PORT env var")
        .parse()
        .unwrap();
    let db_path: String = dotenvy::var("DB_PATH").expect("Please set DB_PATH env var");
    let discord_webhook_url =
        dotenvy::var("DISCORD_WEBHOOK_URL").expect("Please set DISCORD_WEBHOOK_URL env var");
    let openai_client = if let Ok(url) = dotenvy::var("OPENAI_URL")
        && let Ok(model) = dotenvy::var("MODEL")
    {
        let header_kv = dotenvy::var("AUTH").ok().and_then(|v| {
            v.split_once('|')
                .map(|(k, v)| openai_client::OpenAIAuth::ApiKey {
                    key: k.to_string(),
                    value: v.to_string(),
                })
        });
        Some(OpenAIClient::new(url, model, header_kv))
    } else {
        None
    };
    EnvVars {
        username,
        password,
        port,
        db_path,
        openai_client,
        bot_clan_name,
        discord_webhook_url,
    }
});

#[derive(Default, Clone, Component)]
pub struct State {
    pub location: Arc<Mutex<BotLocation>>,
    pub recv_instruction: Arc<Mutex<Option<Receiver<Instruction>>>>,
    pub conn: Arc<Mutex<Option<rusqlite::Connection>>>,
    pub messages: Arc<Mutex<FifoBuffer<1000, FormattedText>>>,
    pub broadcaster: Option<Arc<Broadcaster>>,
    pub bot_activity_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub stay_in_bedawrs_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub banned: Arc<Mutex<bool>>,
    pub banned_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl State {
    #[must_use]
    pub fn new(
        recv_instruction: Receiver<Instruction>,
        conn: rusqlite::Connection,
        messages: Arc<Mutex<FifoBuffer<1000, FormattedText>>>,
        broadcaster: Arc<Broadcaster>,
    ) -> Self {
        Self {
            conn: Arc::new(Mutex::new(Some(conn))),
            recv_instruction: Arc::new(Mutex::new(Some(recv_instruction))),
            messages,
            broadcaster: Some(broadcaster),
            ..Default::default()
        }
    }
}

#[must_use]
pub fn is_spam_message<S: AsRef<str>>(m: S) -> bool {
    m.as_ref().contains("è entrato nella lobby.")
        || m.as_ref().starts_with("CORALMC")
        || m.as_ref().contains("Login eseguito con successo.")
}

#[actix_web::main]
async fn main() -> AppExit {
    let (instruction_sender, instruction_receiver) = tokio::sync::mpsc::channel::<Instruction>(10);
    let instruction_sender = Arc::new(instruction_sender);
    let messages: Arc<Mutex<FifoBuffer<1000, FormattedText>>> =
        Arc::new(Mutex::new(FifoBuffer::new()));
    let connection = instantiate_db(&ENV_VARS.db_path).expect("failed starting db");

    let broadcaster = Arc::new(Broadcaster::new());
    let broadcaster_server = broadcaster.clone();

    let messages_bot = messages.clone();
    let port = ENV_VARS.port;
    let server = tokio::spawn(async move {
        eprintln!("UI available at http://127.0.0.1:{port}/");

        HttpServer::new(move || {
            let messages_clone = messages.clone();
            App::new()
                .app_data(web::Data::new(instruction_sender.clone()))
                .app_data(web::Data::new(MessageLog {
                    messages: messages_clone,
                }))
                .app_data(web::Data::new(broadcaster_server.clone()))
                .service(index)
                .service(ws_handler)
                .service(bot_chat)
                .service(bot_tab)
                .service(bot_jump)
                .service(bot_walk)
                .service(bot_sneak)
                .service(bot_slot)
                .service(bot_get_slot)
                .service(bot_messages)
                .service(bot_agent)
                .service(bot_get_stats)
        })
        .bind(("127.0.0.1", port))
        .expect("server failed")
        .run()
        .await
    });

    let account = Account::offline(&ENV_VARS.username);
    let state = State::new(instruction_receiver, connection, messages_bot, broadcaster);

    let client = ClientBuilder::new()
        .set_handler(handle)
        .set_state(state)
        .reconnect_after(Duration::from_mins(1))
        .start(account, "wind.coralmc.it");

    tokio::select! {
        () = shutdown_signal() => {
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

/// # Errors
///
/// - shouldn't fail, opens db on path and instantiates it if it doesn't exist
pub fn instantiate_db(path: &str) -> rusqlite::Result<rusqlite::Connection> {
    let connection = rusqlite::Connection::open(path)?;
    connection.execute(
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
    )?;
    Ok(connection)
}
