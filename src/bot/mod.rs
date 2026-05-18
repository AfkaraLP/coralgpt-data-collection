pub mod tools;

use std::fmt::Write;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use azalea::client_chat::ChatPacket;
use azalea::{
    WalkDirection, prelude::*, protocol::packets::game::ServerboundGamePacket,
    registry::builtin::ItemKind,
};
use openai_client::OpenAIClient;
use regex::Regex;
use tokio::{sync::mpsc::Sender, time};

use crate::ENV_VARS;
use crate::bot::tools::{
    AcceptFriendRequest, AddFriend, DeclineFriendRequest, GetCurrentPosition, GetInventory,
    GetNearbyBlocks, GetPartyInfo, GetStats, InviteParty, JoinGame, JoinParty, LeaveParty,
    NearbyBlockKinds, PathFind, QuickCommand, ReadChatHistory, ReadTabList, RemoveFriend,
    RespondToUser, RunCommand, SelectHotbarSlot, SendChatMessage, SendDirectMessage,
    SendPartyMessage, Sleep, SpawnSubAgent,
};
use crate::{State, message::Message, server::WsServerMessage};
use openai_client::ToolMap;

const SYSTEM_PROMPT: &str = include_str!("SYSTEM_PROMPT.md");
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
    Agent(String),
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
            let task = tokio::task::spawn(async move {
                let mut interval = time::interval(Duration::from_secs(5));

                loop {
                    interval.tick().await;
                    let is_in_bedwars_lobby = {
                        let location = state.location.lock().await;
                        location.eq(&BotLocation::BedwarsLobby)
                    };
                    if is_in_bedwars_lobby {
                        let task =
                            tokio::task::spawn(
                                async move { try_stay_in_bedwars_lobby(&bot).await },
                            );
                        state.stay_in_bedawrs_task.lock().await.replace(task);
                        break eprintln!("successfully joined the bedwars lobby...");
                    }

                    if let Ok(Some(inventory)) = bot.open_inventory()
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
            state.bot_activity_task.lock().await.replace(task);
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

async fn try_stay_in_bedwars_lobby(bot: &Client) {
    let mut interval = time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        bot.walk(azalea::WalkDirection::Backward);
        bot.jump();
        tokio::time::sleep(Duration::from_secs(3)).await;
        bot.walk(WalkDirection::BackwardLeft);
        tokio::time::sleep(Duration::from_millis(500)).await;
        bot.walk(WalkDirection::BackwardRight);
        tokio::time::sleep(Duration::from_secs(1)).await;
        bot.walk(WalkDirection::BackwardLeft);
        tokio::time::sleep(Duration::from_millis(500)).await;
        bot.walk(WalkDirection::None);

        if let Ok(Some(inventory)) = bot.open_inventory()
            && let Some(slots) = inventory.slots()
            && !slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
            && let Ok(tab_list) = bot.tab_list()
            && !tab_list.into_iter().any(|entry| {
                entry
                    .1
                    .display_name
                    .unwrap_or_default()
                    .to_string()
                    .to_lowercase()
                    .contains(ENV_VARS.bot_clan_name.as_deref().unwrap_or("<UNK>"))
            })
        {
            bot.chat("/hub");
            join_bedwars_lobby(bot).await;
        }
    }
}

static DM_MSG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(Da \w{3,16}\) .*$").unwrap());

pub fn is_dm_message<S: AsRef<str>>(msg: S) -> bool {
    DM_MSG_REGEX.is_match(msg.as_ref())
}

async fn handle_agent_instruction(bot: &Client, state: &State, prompt: &str, username: &str) {
    if let Some(ai_client) = &ENV_VARS.openai_client {
        let subagent_tools = ToolMap::new()
            .register_tool(RespondToUser::new(bot, username, state))
            .register_tool(Sleep::new(state))
            .register_tool(SendDirectMessage::new(bot, state))
            .register_tool(SendChatMessage::new(bot, state))
            .register_tool(QuickCommand::new(bot, state))
            .register_tool(RunCommand::new(bot, state))
            .register_tool(NearbyBlockKinds::new(bot))
            .register_tool(PathFind::new(bot))
            .register_tool(GetCurrentPosition::new(bot))
            .register_tool(GetStats::new(bot, state))
            .register_tool(ReadChatHistory::new(state))
            .register_tool(ReadTabList::new(bot))
            .register_tool(AcceptFriendRequest::new(bot))
            .register_tool(DeclineFriendRequest::new(bot))
            .register_tool(GetInventory::new(bot))
            .register_tool(SelectHotbarSlot::new(bot))
            .register_tool(JoinParty::new(bot, state))
            .register_tool(JoinGame::new(bot, state))
            .register_tool(InviteParty::new(bot, state))
            .register_tool(SendPartyMessage::new(bot, state))
            .register_tool(LeaveParty::new(bot, state))
            .register_tool(GetPartyInfo::new(bot, state))
            .register_tool(AddFriend::new(bot, state))
            .register_tool(GetNearbyBlocks::new(bot))
            .register_tool(RemoveFriend::new(bot, state));
        let subagent_tools = Arc::new(subagent_tools);
        let tools_map = ToolMap::new()
            .register_tool(RespondToUser::new(bot, username, state))
            .register_tool(SpawnSubAgent::new(&subagent_tools));
        let mut system_prompt = format!("{SYSTEM_PROMPT}\n\n## Available tools for the Agent:\n\n");
        for tool in subagent_tools.values() {
            _ = writeln!(
                &mut system_prompt,
                "{}: {}",
                tool.get_name(),
                tool.get_description()
            );
        }
        bot.run_agent_loop(ai_client, &system_prompt, prompt, username, tools_map)
            .await;
    }
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
            Instruction::Sneak(sneaking) => _ = bot.set_crouching(sneaking),
            Instruction::Slot(slot) => bot.set_selected_hotbar_slot(slot),
            Instruction::GetSlot(sender) => {
                let _ = sender
                    .send(bot.selected_hotbar_slot().unwrap_or_default())
                    .await;
            }
            Instruction::Agent(prompt) => {
                handle_agent_instruction(bot, state, &prompt, &ENV_VARS.username).await;
            }
        }
    }
}

async fn send_tab_list(bot: &Client, sender: Sender<String>) {
    let mut players = String::new();
    for (_, player) in bot.tab_list().unwrap_or_default() {
        _ = writeln!(
            &mut players,
            "{}",
            player.display_name.unwrap_or_default().to_html()
        );
    }
    match sender.send(players).await {
        Ok(()) => {}
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
    let Ok(Some(container)) = bot.wait_for_container_open(Some(400)).await else {
        return eprintln!("failed to get open container...");
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    let Some(slots) = container.slots() else {
        return eprintln!("failed to get container slots...");
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    let Some(bed_idx) = slots
        .iter()
        .enumerate()
        .find(|(_idx, item)| item.kind().to_string().to_lowercase().contains("bed"))
        .map(|v| v.0)
    else {
        for (idx, slot) in slots.iter().enumerate() {
            eprintln!("got item: {:#?} [{idx}]", slot.kind());
        }
        return eprintln!("couldn't get the bed idx");
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    container.left_click(bed_idx);
    bot.set_selected_hotbar_slot(3);
}

static DM_PARSE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(Da (\w+)\) (.*)").unwrap());

async fn handle_message_event(bot: &Client, message: ChatPacket, state: &State) {
    println!("{}", message.message().to_ansi());
    handle_register_login(bot, &message, state).await;
    {
        let mut messages = state.messages.lock().await;
        messages.push(message.message());
    }
    if let Some(ref broadcaster) = state.broadcaster {
        broadcaster.send(WsServerMessage::NewMessage(message.message().to_html()));
    }
    let message = message.message().to_string();
    if is_dm_message(message.as_str())
        && let Some(captures) = DM_PARSE_REGEX.captures(&message)
        && let Some(username) = captures.get(1)
        && let Some(message) = &captures.get(2)
        && ["Afkara", "NickAc", "8hi"].contains(&username.as_str())
    {
        handle_admin_dm(message.as_str().trim(), username.as_str(), bot, state).await;
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
        if let Ok(Some(player_uuid)) = bot.player_uuid_by_username(username)
            && let Ok(tab_list) = bot.tab_list()
            && let Some(player) = tab_list.get(&player_uuid)
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
    if message.starts_with("/agent") {
        let prompt = message.strip_prefix("/agent").unwrap_or(message);
        handle_agent_instruction(bot, state, prompt, username).await;
    }
}

static WEBHOOK_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);
async fn notify_roby_join() {
    let body = r##"{"content": "# 📡 **ENTRY DETECTED** <@&1151557434494681139>\n\n### 👤 **User:** `Roby98`\n\n### 🟢 **Status:** `CONNECTED`\n\n> **Note:** The target user **Roby98** has just joined the server!!!"}"##;
    if let Err(e) = WEBHOOK_CLIENT.post("REDACTED").body(body).header("Content-Type", "application/json").send().await {
        eprintln!("failed sending webhook: {e:#?}");
    }
}

async fn handle_register_login(bot: &Client, message: &ChatPacket, state: &State) {
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

pub trait AiFunctionality {
    async fn run_agent_loop(
        &self,
        ai_client: &OpenAIClient,
        system_prompt: &str,
        prompt: &str,
        username: &str,
        tools_map: ToolMap<'_>,
    );
}

#[allow(clippy::too_many_lines)]
impl AiFunctionality for Client {
    async fn run_agent_loop(
        &self,
        ai_client: &OpenAIClient,
        system_prompt: &str,
        prompt: &str,
        username: &str,
        tools: ToolMap<'_>,
    ) {
        println!("running agent loop");
        let messages = vec![
            openai_client::ChatCompletionMessageParam::new_system(system_prompt),
            openai_client::ChatCompletionMessageParam::new_user(prompt),
        ];
        let res = ai_client.run_agent(messages, &tools).await.map_err(|e| {
            self.chat(format!("/msg {username} failed to run agent loop..."));
            println!("{e:#?}");
        });
        if let Ok((res, _)) = res {
            eprintln!(
                "agent loop final response: {}",
                res.content.as_ref().unwrap_or(&String::new())
            );
        }
        self.chat(format!("/msg {username} finished agent loop..."));
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
