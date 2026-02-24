pub mod tools;

use std::fmt::Write;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use azalea::container::ContainerHandle;
use azalea::{
    WalkDirection, prelude::*, protocol::packets::game::ServerboundGamePacket,
    registry::builtin::ItemKind,
};
use regex::Regex;
use tokio::sync::Mutex;
use tokio::{sync::mpsc::Sender, time};

use crate::ENV_VARS;
use crate::agent::openai_client::{ChatCompletionMessageParam, ToolMap};
use crate::bot::tools::{
    GetInventory, InviteParty, JoinParty, ReadChatHistory, ReadTabList, RespondToUser,
    SelectHotbarSlot, SendChatMessage, SendDirectMessage, Sleep, StopAgentLoop,
};
use crate::{State, message::Message, server::WsServerMessage};

const SYSTEM_PROMPT: &str = include_str!("SYSTEM_PROMPT.txt");
#[derive(Default, Clone, Component)]
pub enum Instruction {
    Chat(String),
    #[default]
    Jump,
    TabList(Sender<String>),
    Walk(WalkDirection),
    Sneak(bool),
    Slot(u8),
    GetSlot(Sender<u8>),
}

#[derive(Default, Clone, Component, PartialEq, Eq, PartialOrd, Ord)]
pub enum BotLocation {
    #[default]
    Login,
    Lobby,
    BedwarsLobby,
}

pub async fn handle(bot: Client, event: Event, state: State) -> anyhow::Result<()> {
    match event {
        Event::Spawn => {
            let pwd = &ENV_VARS.password;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            bot.chat(format!("/login {pwd}"));
            tokio::spawn(async move {
                let mut interval = time::interval(Duration::from_secs(5));

                loop {
                    interval.tick().await;
                    let is_in_bedwars_lobby = {
                        let location = state.location.lock().await;
                        location.eq(&BotLocation::BedwarsLobby)
                    };
                    if is_in_bedwars_lobby {
                        tokio::spawn(async move {
                            let mut interval = time::interval(Duration::from_secs(30));
                            loop {
                                interval.tick().await;
                                bot.walk(azalea::WalkDirection::Backward);
                                bot.jump();
                                tokio::time::sleep(Duration::from_secs(3)).await;
                                bot.walk(WalkDirection::BackwardLeft);
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                bot.walk(WalkDirection::BackwardRight);
                                tokio::time::sleep(Duration::from_millis(1000)).await;
                                bot.walk(WalkDirection::BackwardLeft);
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                bot.walk(WalkDirection::None);

                                let mut inventory: Option<ContainerHandle> = None;
                                _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                                    if let Some(opened_inv) = bot.open_inventory() {
                                        inventory.replace(opened_inv);
                                    }
                                }));

                                if let Some(inventory) = inventory
                                    && let Some(slots) = inventory.slots()
                                    && !slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
                                    && !bot.tab_list().into_iter().any(|entry| {
                                        entry
                                            .1
                                            .display_name
                                            .unwrap_or_default()
                                            .to_string()
                                            .to_lowercase()
                                            .contains("[CoralProxy]")
                                    })
                                {
                                    bot.chat("/hub");
                                    join_bedwars_lobby(&bot).await;
                                }
                            }
                        });
                        break eprintln!("successfully joined the bedwars lobby...");
                    }

                    let mut inventory: Option<ContainerHandle> = None;
                    _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        if let Some(opened_inv) = bot.open_inventory() {
                            inventory.replace(opened_inv);
                        }
                    }));

                    if let Some(inventory) = inventory
                        && let Some(slots) = inventory.slots()
                        && slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
                    {
                        let mut location = state.location.lock().await;
                        *location = BotLocation::BedwarsLobby;
                    }
                    if state.location.lock().await.eq(&BotLocation::Lobby) {
                        join_bedwars_lobby(&bot).await;
                    }
                }
            });
        }
        Event::Chat(message) => {
            handle_message_event(&bot, message, &state).await;
        }
        Event::Tick => {
            handle_instruction(&bot, &state).await;
        }
        _ => {}
    }

    Ok(())
}

static DM_MSG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(Da \w{3,16}\) .*$").unwrap());
pub fn is_dm_message(msg: impl AsRef<str>) -> bool {
    DM_MSG_REGEX.is_match(msg.as_ref())
}

async fn handle_instruction(bot: &Client, state: &State) {
    let mut guard = state.recv_instruction.lock().await;
    if let Some(recv) = guard.as_mut()
        && let Some(instruction) = recv.recv().await
    {
        match instruction {
            Instruction::Chat(message) => bot.chat(message),
            Instruction::Jump => bot.jump(),
            Instruction::TabList(sender) => send_tab_list(bot, sender).await,
            Instruction::Walk(direction) => bot.walk(direction),
            Instruction::Sneak(sneaking) => bot.set_crouching(sneaking),
            Instruction::Slot(slot) => bot.set_selected_hotbar_slot(slot),
            Instruction::GetSlot(sender) => {
                let _ = sender.send(bot.selected_hotbar_slot()).await;
            }
        }
    }
}

async fn send_tab_list(bot: &Client, sender: Sender<String>) {
    let mut players = String::new();
    for (_, player) in bot.tab_list() {
        _ = writeln!(
            &mut players,
            "{}",
            player.display_name.unwrap_or_default().to_html()
        );
    }
    match sender.send(players).await {
        Ok(()) => eprintln!("successfully sent tablist..."),
        Err(_) => eprintln!("failed to send tablist..."),
    }
}

async fn join_bedwars_lobby(bot: &Client) {
    bot.set_selected_hotbar_slot(4);
    tokio::time::sleep(Duration::from_secs(1)).await;
    bot.wait_updates(1).await;

    bot.right_click();
    tokio::time::sleep(Duration::from_secs(1)).await;
    bot.wait_updates(1).await;
    if let Some(container) = bot.wait_for_container_open(Some(400)).await
        && let Some(slots) = container.slots()
    {
        let Some(bed_idx) = slots
            .iter()
            .enumerate()
            .find(|(_idx, item)| item.kind().eq(&ItemKind::RedBed))
            .map(|v| v.0)
        else {
            return eprintln!("couldn't get the bed idx");
        };
        tokio::time::sleep(Duration::from_millis(1000)).await;
        container.left_click(bed_idx);
        bot.set_selected_hotbar_slot(3);
    } else {
        eprintln!("failed to get open container...");
    }
}

async fn handle_message_event(bot: &Client, message: azalea::chat::ChatPacket, state: &State) {
    println!("{}", message.message().to_ansi());
    handle_register_login(bot, &message, state).await;
    {
        let mut messages = state.messages.lock().await;
        messages.push(message.message());
    }
    if let Some(ref broadcaster) = state.broadcaster {
        broadcaster.send(WsServerMessage::NewMessage(message.message()));
    }
    let message = message.message().to_string();
    if is_dm_message(&message) {
        let re = Regex::new(r"\(Da (\w+)\) (.*)");
        if let Ok(re) = re
            && let Some(captures) = re.captures(&message)
            && let Some(username) = captures.get(1)
            && let Some(message) = &captures.get(2)
            && ["Afkara", "NickAc", "8hi"].contains(&username.as_str())
        {
            handle_admin_dm(message.as_str().trim(), username.as_str(), bot, state).await;
        }
    }
    if message.contains("Roby98 è entrato nella lobby.") {
        notify_roby_join().await;
    }
    if message.contains("La partita inizia tra") {
        bot.chat("/l");
    }
    if message.to_lowercase().trim().starts_with('[')
        && let Some(mut message) = Message::parse(&message)
        && let Some(conn) = state.conn.lock().await.as_ref()
    {
        let username = message.name.as_str();
        if let Some(player_uuid) = bot.player_uuid_by_username(username)
            && let Some(player) = bot.tab_list().get(&player_uuid)
        {
            message = message.set_ping(player.latency.into());
        }
        match message.insert(conn) {
            Ok(()) => {}
            Err(_) => eprintln!("failed writing to db..."),
        }
    }
}

async fn handle_admin_dm(message: &str, username: &str, bot: &Client, state: &State) {
    if let Some(ai_client) = &ENV_VARS.openai_client {
        handle_ai_admin_dm(message, username, bot, state, ai_client).await;
    }
}

async fn handle_ai_admin_dm(
    message: &str,
    username: &str,
    bot: &Client,
    state: &State,
    ai_client: &crate::agent::openai_client::OpenAIClient,
) {
    let system_promt = ChatCompletionMessageParam {
        content: SYSTEM_PROMPT.into(),
        role: crate::agent::openai_client::AIChatRole::System,
    };
    let should_stop_agent_loop = Arc::new(Mutex::new(false));
    let tools_map = ToolMap::new()
        .register_tool(SendDirectMessage::new(bot, state))
        .register_tool(ReadChatHistory::new(state))
        .register_tool(ReadTabList::new(bot))
        .register_tool(SendChatMessage::new(bot, state))
        .register_tool(RespondToUser::new(bot, username, state))
        .register_tool(GetInventory::new(bot))
        .register_tool(SelectHotbarSlot::new(bot))
        .register_tool(JoinParty::new(bot))
        .register_tool(InviteParty::new(bot))
        .register_tool(StopAgentLoop::new(should_stop_agent_loop.clone()))
        .register_tool(Sleep::new(state));

    let mut messages = Vec::new();
    messages.push(system_promt);
    if message.starts_with("/agent") {
        let user_message = ChatCompletionMessageParam {
            content: message
                .strip_prefix("/agent")
                .unwrap_or_default()
                .to_string(),
            role: crate::agent::openai_client::AIChatRole::User,
        };
        messages.push(user_message);

        while !*should_stop_agent_loop.lock().await {
            if messages.len() > 15 {
                let mut compaction_its = 0_usize;
                let compacted = loop {
                    if let Some(compacted) = ai_client.compact_history(&messages).await {
                        break compacted;
                    }
                    if compaction_its > 10 {
                        break messages
                            .get(1)
                            .map_or("just stop and message me you stopped".into(), |v| {
                                v.content.clone()
                            });
                    }
                    compaction_its += 1;
                };
                messages = vec![
                    ChatCompletionMessageParam {
                        content: SYSTEM_PROMPT.to_string(),
                        role: crate::agent::openai_client::AIChatRole::System,
                    },
                    ChatCompletionMessageParam {
                        content: compacted,
                        role: crate::agent::openai_client::AIChatRole::User,
                    },
                ];
            }
            let comp = ai_client.get_completion(&messages, &tools_map).await;
            let Ok(completion) = comp else {
                break;
            };
            let Some(first) = completion.choices.first() else {
                break;
            };
            let Some(ref tool_calls) = first.message.tool_calls else {
                break;
            };
            let agent_response_content = first.message.content.clone().unwrap_or_default();
            if !agent_response_content.is_empty() || tool_calls.is_empty() {
                messages.push(ChatCompletionMessageParam {
                    content: agent_response_content,
                    role: crate::agent::openai_client::AIChatRole::Assistant,
                });
            }

            let mut tool_responses = String::new();
            for tool_call in tool_calls {
                if let Some(tool) = tools_map.get(&tool_call.function.name) {
                    let answer = tool
                        .invoke(
                            &serde_json::from_str(&tool_call.function.arguments)
                                .unwrap_or_default(),
                        )
                        .await;
                    tokio::time::sleep(tool.get_timeout_wait()).await;
                    _ = writeln!(
                        &mut tool_responses,
                        "tool({}) output: \n\n{answer}",
                        tool.get_name()
                    );
                }
            }
            if !tool_responses.is_empty() {
                messages.push(ChatCompletionMessageParam {
                    content: tool_responses,
                    role: crate::agent::openai_client::AIChatRole::Tool,
                });
            }

            println!("[LLM] we sent messages: \n\n{messages:#?}");
        }
    }
}

async fn notify_roby_join() {
    let client = reqwest::Client::new();
    let body = r##"{"content": "# 📡 **ENTRY DETECTED** <@&1151557434494681139>\n\n### 👤 **User:** `Roby98`\n\n### 🟢 **Status:** `CONNECTED`\n\n> **Note:** The target user **Roby98** has just joined the server!!!"}"##;
    if let Err(e) = client.post("REDACTED").body(body).header("Content-Type", "application/json").send().await {
        eprintln!("failed sending webhook: {e:#?}");
    }
}

async fn handle_register_login(bot: &Client, message: &azalea::chat::ChatPacket, state: &State) {
    let pwd = &ENV_VARS.password;
    if message
        .message()
        .to_string()
        .trim()
        .contains("CORALMC » Per favore, esegui la registrazione con il comando: /register [password] [confermaPassword]")
    {
        eprintln!("registering...");
        bot.chat(format!("/register {pwd} {pwd}"));
    }
    if message
        .message()
        .to_string()
        .trim()
        .contains("CORALMC » Per favore, esegui l'autenticazione con il comando: /login [password]")
    {
        eprintln!("logging in...");
        bot.chat(format!("/login {pwd}"));
    }

    if message
        .message()
        .to_string()
        .trim()
        .contains("Login eseguito con successo.")
    {
        eprintln!("setting location to lobby...");
        let mut location = state.location.lock().await;
        *location = BotLocation::Lobby;
    }
}

pub trait RightClick {
    fn right_click(&self);
}

impl RightClick for Client {
    fn right_click(&self) {
        self.write_packet(ServerboundGamePacket::UseItem(
            azalea::protocol::packets::game::ServerboundUseItem {
                hand: azalea::protocol::packets::game::s_interact::InteractionHand::MainHand,
                seq: 0,
                y_rot: 0.0,
                x_rot: 0.0,
            },
        ));
    }
}
