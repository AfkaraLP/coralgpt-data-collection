use actix::{ActorContext, ActorFutureExt, AsyncContext, WrapFuture};
use actix_web::{HttpRequest, HttpResponse, Responder, get, post, rt, web};
use actix_web_actors::ws;
use azalea::{FormattedText, WalkDirection};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{
    Mutex, broadcast,
    mpsc::{self, Sender},
};

use crate::ENV_VARS;
use crate::{bot::Instruction, buffer::FifoBuffer};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_TIMEOUT: Duration = Duration::from_mins(1);

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
pub enum WsClientMessage {
    #[serde(rename = "chat")]
    Chat(String),
    #[serde(rename = "jump")]
    Jump,
    #[serde(rename = "tab")]
    Tab,
    #[serde(rename = "walk")]
    Walk(String),
    #[serde(rename = "sneak")]
    Sneak(bool),
    #[serde(rename = "slot")]
    Slot(u8),
    #[serde(rename = "getSlot")]
    GetSlot,
    #[serde(rename = "getMessages")]
    GetMessages,
}

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type", content = "data")]
pub enum WsServerMessage {
    #[serde(rename = "messages")]
    Messages(Vec<String>),
    #[serde(rename = "newMessage")]
    NewMessage(String),
    #[serde(rename = "players")]
    Players(Vec<String>),
    #[serde(rename = "slot")]
    Slot(u8),
    #[serde(rename = "result")]
    Result { success: bool, message: String },
}

pub struct Broadcaster {
    pub tx: broadcast::Sender<WsServerMessage>,
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Broadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self { tx }
    }

    pub fn send(&self, msg: WsServerMessage) {
        let _ = self.tx.send(msg);
    }
}

pub struct WsSession {
    heartbeat: Instant,
    instruction_tx: Arc<Sender<Instruction>>,
    messages: Arc<Mutex<FifoBuffer<1000, FormattedText>>>,
    broadcaster: Arc<Broadcaster>,
    broadcast_task: Option<tokio::task::JoinHandle<()>>,
}

impl WsSession {
    pub fn new(
        instruction_tx: Arc<Sender<Instruction>>,
        messages: Arc<Mutex<FifoBuffer<1000, FormattedText>>>,
        broadcaster: Arc<Broadcaster>,
    ) -> Self {
        Self {
            heartbeat: Instant::now(),
            instruction_tx,
            messages,
            broadcaster,
            broadcast_task: None,
        }
    }

    fn heartbeat(ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }

    fn send_json(ctx: &mut ws::WebsocketContext<Self>, msg: &WsServerMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            ctx.text(json);
        }
    }

    fn handle_message(&mut self, text: &str, ctx: &mut ws::WebsocketContext<Self>) {
        if let Ok(client_msg) = serde_json::from_str::<WsClientMessage>(text) {
            let tx = self.instruction_tx.clone();

            match client_msg {
                WsClientMessage::Chat(msg) => {
                    handle_client_chat(ctx, &tx, msg);
                }
                WsClientMessage::Jump => {
                    handle_client_jump(ctx, &tx);
                }
                WsClientMessage::Tab => {
                    self.handle_client_tab(ctx, &tx);
                }
                WsClientMessage::Walk(direction) => {
                    handle_client_walk(&tx, &direction);
                }
                WsClientMessage::Sneak(sneaking) => {
                    handle_client_sneak(ctx, &tx, sneaking);
                }
                WsClientMessage::Slot(slot) => {
                    handle_client_set_slot(ctx, &tx, slot);
                }
                WsClientMessage::GetSlot => {
                    self.handle_client_get_slot(ctx, &tx);
                }
                WsClientMessage::GetMessages => {
                    self.handle_client_get_messages(ctx);
                }
            }
        } else {
            Self::send_json(
                ctx,
                &WsServerMessage::Result {
                    success: false,
                    message: "invalid message format".into(),
                },
            );
        }
    }

    fn handle_client_get_messages(&mut self, ctx: &mut ws::WebsocketContext<WsSession>) {
        let messages = self.messages.clone();
        ctx.spawn(
            async move {
                let msgs = messages.lock().await;
                let messages_vec: Vec<String> =
                    msgs.iter().rev().map(FormattedText::to_html).collect();
                WsServerMessage::Messages(messages_vec)
            }
            .into_actor(self)
            .map(|msg, _act, ctx| {
                if let Ok(json) = serde_json::to_string(&msg) {
                    ctx.text(json);
                }
            }),
        );
    }

    fn handle_client_get_slot(
        &mut self,
        ctx: &mut ws::WebsocketContext<WsSession>,
        tx: &Sender<Instruction>,
    ) {
        let tx = tx.clone();
        ctx.spawn(
            async move {
                let (resp_tx, mut resp_rx) = mpsc::channel::<u8>(1);
                if tx.send(Instruction::GetSlot(resp_tx)).await.is_ok()
                    && let Some(slot) = resp_rx.recv().await
                {
                    return Some(WsServerMessage::Slot(slot));
                }
                None
            }
            .into_actor(self)
            .map(|msg, _act, ctx| {
                if let Some(msg) = msg
                    && let Ok(json) = serde_json::to_string(&msg)
                {
                    ctx.text(json);
                }
            }),
        );
    }

    fn handle_client_tab(
        &mut self,
        ctx: &mut ws::WebsocketContext<WsSession>,
        tx: &Arc<Sender<Instruction>>,
    ) {
        let tx = tx.clone();
        ctx.spawn(
            async move {
                let (resp_tx, mut resp_rx) = mpsc::channel::<String>(1);
                if tx.send(Instruction::TabList(resp_tx)).await.is_ok()
                    && let Some(tablist) = resp_rx.recv().await
                {
                    let players: Vec<String> = tablist
                        .split('\n')
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                        .collect();
                    return Some(WsServerMessage::Players(players));
                }

                None
            }
            .into_actor(self)
            .map(|msg, _act, ctx| {
                if let Some(msg) = msg
                    && let Ok(json) = serde_json::to_string(&msg)
                {
                    ctx.text(json);
                }
            }),
        );
    }
}

fn handle_client_set_slot(
    ctx: &mut ws::WebsocketContext<WsSession>,
    tx: &Arc<Sender<Instruction>>,
    slot: u8,
) {
    if slot <= 8 {
        let tx = tx.clone();
        rt::spawn(async move {
            let _ = tx.send(Instruction::Slot(slot)).await;
        });
        WsSession::send_json(ctx, &WsServerMessage::Slot(slot));
    }
}

fn handle_client_sneak(
    ctx: &mut ws::WebsocketContext<WsSession>,
    tx: &Arc<Sender<Instruction>>,
    sneaking: bool,
) {
    let tx = tx.clone();
    rt::spawn(async move {
        let _ = tx.send(Instruction::Sneak(sneaking)).await;
    });
    WsSession::send_json(
        ctx,
        &WsServerMessage::Result {
            success: true,
            message: if sneaking {
                "sneaking"
            } else {
                "stopped sneaking"
            }
            .into(),
        },
    );
}

fn handle_client_walk(tx: &Arc<Sender<Instruction>>, direction: &str) {
    let dir = match direction {
        "forward" => Some(WalkDirection::Forward),
        "backward" => Some(WalkDirection::Backward),
        "left" => Some(WalkDirection::Left),
        "right" => Some(WalkDirection::Right),
        "forward-left" => Some(WalkDirection::ForwardLeft),
        "forward-right" => Some(WalkDirection::ForwardRight),
        "backward-left" => Some(WalkDirection::BackwardLeft),
        "backward-right" => Some(WalkDirection::BackwardRight),
        "stop" => Some(WalkDirection::None),
        _ => None,
    };
    if let Some(d) = dir {
        let tx = tx.clone();
        rt::spawn(async move {
            let _ = tx.send(Instruction::Walk(d)).await;
        });
    }
}

fn handle_client_jump(ctx: &mut ws::WebsocketContext<WsSession>, tx: &Arc<Sender<Instruction>>) {
    let tx = tx.clone();
    rt::spawn(async move {
        let _ = tx.send(Instruction::Jump).await;
    });
    WsSession::send_json(
        ctx,
        &WsServerMessage::Result {
            success: true,
            message: "jumped".into(),
        },
    );
}

fn handle_client_chat(
    ctx: &mut ws::WebsocketContext<WsSession>,
    tx: &Arc<Sender<Instruction>>,
    msg: String,
) {
    let tx = tx.clone();
    rt::spawn(async move {
        let _ = tx.send(Instruction::Chat(msg)).await;
    });
    WsSession::send_json(
        ctx,
        &WsServerMessage::Result {
            success: true,
            message: "sent".into(),
        },
    );
}

impl actix::Actor for WsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        Self::heartbeat(ctx);
        ctx.set_mailbox_capacity(20);

        let mut rx = self.broadcaster.tx.subscribe();
        let addr = ctx.address();
        self.broadcast_task = Some(tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&msg) {
                    addr.do_send(WsBroadcast(json));
                }
            }
        }));
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        if let Some(task) = self.broadcast_task.take() {
            task.abort();
        }
    }
}

#[derive(actix::Message)]
#[rtype(result = "()")]
struct WsBroadcast(String);

impl actix::Handler<WsBroadcast> for WsSession {
    type Result = ();
    fn handle(&mut self, msg: WsBroadcast, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

impl actix::StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                self.handle_message(&text, ctx);
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => {}
        }
    }
}

#[get("/ws")]
pub async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
    instruction_tx: web::Data<Arc<Sender<Instruction>>>,
    message_log: web::Data<MessageLog>,
    broadcaster: web::Data<Arc<Broadcaster>>,
) -> Result<HttpResponse, actix_web::Error> {
    let session = WsSession::new(
        instruction_tx.get_ref().clone(),
        message_log.messages.clone(),
        broadcaster.get_ref().clone(),
    );
    ws::start(session, &req, stream)
}

pub struct MessageLog {
    pub messages: Arc<Mutex<FifoBuffer<1000, FormattedText>>>,
}

const HTML: &str = include_str!("../index.html");

#[get("/")]
pub async fn index() -> impl Responder {
    HttpResponse::Ok().content_type("text/html").body(HTML)
}

#[post("/chat")]
pub async fn bot_chat(
    req_body: String,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    match data.send(Instruction::Chat(req_body)).await {
        Ok(()) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/jump")]
pub async fn bot_jump(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    match data.send(Instruction::Jump).await {
        Ok(()) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/tab")]
pub async fn bot_tab(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    let (tx, mut rx) = mpsc::channel::<String>(1);
    match data.send(Instruction::TabList(tx)).await {
        Ok(()) => match rx.recv().await {
            Some(tablist) => tablist,
            None => "failed receiving tablist".to_string(),
        },
        Err(_) => "failed sending tab instruction".to_string(),
    }
}

#[get("/messages")]
pub async fn bot_messages(data: web::Data<MessageLog>) -> impl Responder {
    let messages = data.messages.lock().await;
    messages.iter().fold(String::new(), |mut acc, s| {
        _ = write!(&mut acc, "{s}");
        acc
    })
}

#[post("/walk/{direction}")]
pub async fn bot_walk(
    path: web::Path<String>,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    let direction = match path.as_str() {
        "forward" => WalkDirection::Forward,
        "backward" => WalkDirection::Backward,
        "left" => WalkDirection::Left,
        "right" => WalkDirection::Right,
        "forward-left" => WalkDirection::ForwardLeft,
        "forward-right" => WalkDirection::ForwardRight,
        "backward-left" => WalkDirection::BackwardLeft,
        "backward-right" => WalkDirection::BackwardRight,
        "stop" => WalkDirection::None,
        _ => return "invalid direction",
    };
    match data.send(Instruction::Walk(direction)).await {
        Ok(()) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/sneak/{state}")]
pub async fn bot_sneak(
    path: web::Path<String>,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    let sneaking = match path.as_str() {
        "on" => true,
        "off" => false,
        _ => return "invalid state",
    };
    match data.send(Instruction::Sneak(sneaking)).await {
        Ok(()) => "successfully sent instruction",
        Err(_) => "failed",
    }
}

#[post("/slot/{slot}")]
pub async fn bot_slot(
    path: web::Path<u8>,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    let slot = path.into_inner();
    if slot > 8 {
        return "invalid slot (0-8)".to_string();
    }
    match data.send(Instruction::Slot(slot)).await {
        Ok(()) => "successfully sent instruction".to_string(),
        Err(_) => "failed".to_string(),
    }
}

#[get("/slot")]
pub async fn bot_get_slot(data: web::Data<Arc<Sender<Instruction>>>) -> impl Responder {
    let (tx, mut rx) = mpsc::channel::<u8>(1);
    match data.send(Instruction::GetSlot(tx)).await {
        Ok(()) => match rx.recv().await {
            Some(slot) => slot.to_string(),
            None => "failed".to_string(),
        },
        Err(_) => "failed".to_string(),
    }
}

#[derive(Deserialize)]
pub struct AgentRequest {
    prompt: String,
}

#[post("/agent")]
pub async fn bot_agent(
    req: web::Json<AgentRequest>,
    data: web::Data<Arc<Sender<Instruction>>>,
) -> impl Responder {
    match data.send(Instruction::Agent(req.prompt.clone())).await {
        Ok(()) => format!("agent started for user {}", &ENV_VARS.username),
        Err(_) => "failed".to_string(),
    }
}
