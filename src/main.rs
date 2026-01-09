//! A bot that logs chat messages and the number that we've received to the console.

use std::sync::Arc;

use actix_web::{App, HttpServer, web};
use azalea::{ClientBuilder, app::AppExit, prelude::Account};
use tokio::sync::Mutex;

use crate::{
    bot::{Instruction, State, handle},
    server::{bot_chat, bot_jump, bot_tab},
};

mod bot;
mod server;

pub const USERNAME: &str = "CoralGPTv0d1";
pub const PWD: &str = "REDACTED";

use tokio::signal;

#[tokio::main]
async fn main() -> AppExit {
    let (tx, rx) = tokio::sync::mpsc::channel::<Instruction>(10);
    let tx = Arc::new(tx);

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
    let state = State {
        recv_instruction: Arc::new(Mutex::new(Some(rx))),
        ..Default::default()
    };

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
