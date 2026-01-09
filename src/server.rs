use actix_web::{Responder, post, web};
use std::sync::Arc;
use tokio::sync::mpsc::{self, Sender};

use crate::bot::Instruction;

#[post("/chat")]
pub async fn bot_chat(
    req_body: String,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    match data.send(Instruction::Chat(req_body)).await {
        Ok(_) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/jump")]
pub async fn bot_jump(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    match data.send(Instruction::Jump).await {
        Ok(_) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/tab")]
pub async fn bot_tab(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    // let (tx, mut rx) = mpsc::channel::<String>(1);
    match data.send(Instruction::TabList).await {
        Ok(_) => "Sent instruction".to_string(),
        // match rx.recv().await {
        //             Some(tablist) => tablist,
        //             None => "failed receiving tablist".to_string(),
        //         }
        Err(_) => "failed sending tab instruction".to_string(),
    }
}
