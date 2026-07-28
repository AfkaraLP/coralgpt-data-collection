pub mod stats;
pub mod tools;

use std::fmt::Write;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Context, anyhow};
use azalea::BlockPos;
use azalea::client_chat::ChatPacket;
use azalea::pathfinder::PathfinderOpts;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::{
    WalkDirection, prelude::*, protocol::packets::game::ServerboundGamePacket,
    registry::builtin::ItemKind,
};
use openai_client::{ChatCompletionMessageParam, OpenAIClient};
use regex::Regex;
use serde::Serialize;
use tokio::sync::{Mutex, mpsc::Sender};
use tokio::task;
use tokio::time;
use tokio::time::{sleep, timeout};

use crate::ENV_VARS;
use crate::bot::stats::{PlayerStats, get_stats, write_player_stats_markdown};
use crate::bot::tools::{
    AcceptFriendRequest, AddFriend, DeclineFriendRequest, GetCurrentPosition, GetInventory,
    GetNearbyBlocks, GetPartyInfo, GetStats, GoalCompleted, GoalNotCompleted, InviteParty,
    JoinGame, JoinParty, LeaveParty, NearbyBlockKinds, PathFind, QuickCommand, ReadChatHistory,
    ReadTabList, RemoveFriend, RespondToUser, RunCommand, SelectHotbarSlot, SendChatMessage,
    SendDirectMessage, SendPartyMessage, Sleep, SpawnSubAgent,
};
use crate::{State, message::Message, server::WsServerMessage};
use openai_client::ToolMap;

const SYSTEM_PROMPT: &str = include_str!("../../prompts/SYSTEM_PROMPT.md");
const GOAL_ENHANCER: &str = include_str!("../../prompts/GOAL_ENHANCER.md");
const GOAL_VERIFIER: &str = include_str!("../../prompts/GOAL_VERIFIER.md");
// const GOAL_SENTIMENT: &str = include_str!("../../prompts/GOAL_SENTIMENT.md");
const GOAL_REFINER: &str = include_str!("../../prompts/GOAL_REFINER.md");
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
    GetStats(String, Sender<PlayerStats>),
}

#[derive(Default, Clone, Component, PartialEq, Eq, PartialOrd, Ord)]
pub enum BotLocation {
    #[default]
    Login,
    Lobby,
    BedwarsLobby,
}

pub async fn abort_task(slot: &tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>) {
    let old_task = slot.lock().await.take();
    if let Some(task) = old_task {
        task.abort();
    }
}

pub async fn replace_task(
    slot: &tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    new_task: tokio::task::JoinHandle<()>,
) {
    let old = slot.lock().await.replace(new_task);
    if let Some(old) = old {
        old.abort();
    }
}

pub async fn handle(bot: Client, event: Event, state: State) -> anyhow::Result<()> {
    match event {
        Event::Spawn => {
            let pwd = &ENV_VARS.password;
            sleep(Duration::from_millis(500)).await;
            bot.chat(format!("/login {pwd}"));
            replace_task(
                &state.bot_activity_task,
                task::spawn(async move {
                    let mut interval = time::interval(Duration::from_secs(5));

                    loop {
                        interval.tick().await;
                        let is_in_bedwars_lobby: bool = {
                            let location = state.location.lock().await;
                            location.eq(&BotLocation::BedwarsLobby)
                        };
                        if is_in_bedwars_lobby {
                            replace_task(
                                &state.stay_in_bedawrs_task,
                                task::spawn(async move { try_stay_in_bedwars_lobby(&bot).await }),
                            )
                            .await;
                            break eprintln!("successfully joined the bedwars lobby...");
                        }

                        if let Ok(Some(inventory)) = bot.open_inventory()
                            && let Some(slots) = inventory.slots()
                            && slots.iter().any(|item| item.kind().eq(&ItemKind::Anvil))
                        {
                            let mut location = state.location.lock().await;
                            *location = BotLocation::BedwarsLobby;
                        }
                        if state.location.lock().await.eq(&BotLocation::Lobby)
                            && !*state.banned.lock().await
                        {
                            join_bedwars_lobby(&bot).await;
                        }
                    }
                }),
            )
            .await;
        }
        Event::Chat(message) => {
            handle_message_event(&bot, message, &state).await;
        }
        Event::Tick => {
            handle_instruction(&bot, &state).await;
            if *state.banned.lock().await {
                abort_task(&state.stay_in_bedawrs_task).await;
                abort_task(&state.bot_activity_task).await;
                let mut activity = state.banned_task.lock().await;
                if activity.is_none() {
                    activity.replace(task::spawn(async move { do_banned_mining(&bot).await }));
                }
            }
        }
        _ => {}
    }

    Ok(())
}

async fn do_banned_mining(bot: &Client) {
    eprintln!("Pathfinding...");
    loop {
        let first_pos = BlockPos::new(-34, 95, 55);
        bot.goto_with_opts(
            BlockPosGoal(first_pos),
            PathfinderOpts::new()
                .allow_mining(false)
                .max_timeout(Duration::from_secs(30)),
        )
        .await;
        for x in -34..=34 {
            let range: Vec<i32> = if x % 2 == 0 {
                (55..=123).collect()
            } else {
                (55..=123).rev().collect()
            };
            for z in range {
                let y = 94;
                let pathfindpos = BlockPos::new(x, y, z);
                bot.goto_with_opts(
                    BlockPosGoal(pathfindpos),
                    PathfinderOpts::new()
                        .allow_mining(true)
                        .max_timeout(Duration::from_secs(2)),
                )
                .await;
            }
        }
    }
}

async fn try_stay_in_bedwars_lobby(bot: &Client) {
    let mut interval = time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        bot.walk(azalea::WalkDirection::Backward);
        bot.jump();
        sleep(Duration::from_secs(3)).await;
        bot.walk(WalkDirection::BackwardLeft);
        sleep(Duration::from_millis(500)).await;
        bot.walk(WalkDirection::BackwardRight);
        sleep(Duration::from_secs(1)).await;
        bot.walk(WalkDirection::BackwardLeft);
        sleep(Duration::from_millis(500)).await;
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
        let mut system_prompt =
            format!("{SYSTEM_PROMPT}\n\n## Available tools for the Subagent:\n\n");
        for tool in subagent_tools.values() {
            _ = writeln!(
                &mut system_prompt,
                "{}: {}",
                tool.get_name(),
                tool.get_description()
            );
        }
        _ = bot
            .run_agent_loop(ai_client, &system_prompt, prompt, username, tools_map)
            .await;
    }
}

async fn handle_instruction(bot: &Client, state: &State) {
    let mut guard = state.recv_instruction.lock().await;

    let Some(recv) = guard.as_mut() else { return };
    let Ok(Some(instruction)) = timeout(Duration::from_secs(1), recv.recv()).await else {
        return;
    };
    drop(guard);

    match instruction {
        Instruction::Chat(message) => bot.chat(message),
        Instruction::Jump => bot.jump(),
        Instruction::TabList(sender) => send_tab_list(bot, sender).await,
        Instruction::Walk(direction) => bot.walk(direction),
        Instruction::Sneak(sneaking) => _ = bot.set_crouching(sneaking),
        Instruction::Slot(slot) => bot.set_selected_hotbar_slot(slot),
        Instruction::GetSlot(sender) => {
            _ = sender
                .send(bot.selected_hotbar_slot().unwrap_or_default())
                .await;
        }
        Instruction::Agent(prompt) => {
            handle_agent_instruction(bot, state, &prompt, &ENV_VARS.username).await;
        }
        Instruction::GetStats(name, sender) => {
            let player_stats = get_stats(&name, bot, state).await;
            if let Err(e) = sender.send(player_stats).await {
                eprintln!("failed to send player stats: {e:#?}");
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
    if sender.send(players).await.is_err() {
        eprintln!("failed to send tablist...");
    }
}

async fn join_bedwars_lobby(bot: &Client) {
    eprintln!("Joining bedwars lobby...");
    bot.set_selected_hotbar_slot(4);
    sleep(Duration::from_secs(1)).await;
    bot.wait_updates(1).await;

    if let Ok(Some(inv)) = bot.open_inventory()
        && let Ok(Some(menu)) = inv.menu()
        && menu
            .slots()
            .iter()
            .any(|slot| slot.kind() == ItemKind::Anvil)
    {
        return eprintln!("already in bedwars lobby, cancelling join.");
    }

    bot.right_click();
    sleep(Duration::from_secs(1)).await;
    bot.wait_updates(1).await;
    let Ok(Some(container)) = bot.wait_for_container_open(Some(400)).await else {
        return eprintln!("failed to get open container...");
    };
    sleep(Duration::from_secs(1)).await;
    let Some(slots) = container.slots() else {
        return eprintln!("failed to get container slots...");
    };
    sleep(Duration::from_secs(1)).await;
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
    sleep(Duration::from_secs(1)).await;
    container.left_click(bed_idx);
    bot.set_selected_hotbar_slot(3);
}

static DM_PARSE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(Da (\w+)\) (.*)").unwrap());

async fn handle_message_event(bot: &Client, message: ChatPacket, state: &State) {
    // println!("{}", message.message().to_ansi());
    handle_register_login(bot, &message, state).await;
    if message.content().contains("SEI STATO BANNATO!") {
        let mut banned = state.banned.lock().await;
        eprintln!("Bot is banned...");
        *banned = true;
        drop(banned);
        eprintln!("Bot is bannet set to {}", *state.banned.lock().await);
    }

    let mut messages = state.messages.lock().await;
    messages.push(message.message());
    drop(messages);

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
        notify_roby_join(bot, state).await;
    }
    if ["/8)!", "/12)!", "/16)!"]
        .iter()
        .any(|v| message.trim().ends_with(v))
        || message.contains("La partita inizia tra")
    {
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
        if message.insert(conn).is_err() {
            eprintln!("failed writing to db...");
        }
    }
}

async fn handle_admin_dm(message: &str, username: &str, bot: &Client, state: &State) {
    if message.starts_with("/agent") {
        let prompt = message.strip_prefix("/agent").unwrap_or(message);
        handle_agent_instruction(bot, state, prompt, username).await;
    }

    if let Some(openai_client) = &ENV_VARS.openai_client
        && message.starts_with("/goal")
    {
        let goal = message.strip_prefix("/goal").unwrap_or(message);
        run_goal_loop(username, bot, state, goal, openai_client).await;
    }
}

async fn run_goal_loop(
    username: &str,
    bot: &Client,
    state: &State,
    goal: &str,
    openai_client: &OpenAIClient,
) {
    let system_prompt = GOAL_ENHANCER;
    let prompt = format!(
        "Convert this raw goal into a concise 2-3 sentence agent prompt. Be specific about what to do and how to confirm completion. Goal: {goal}"
    );
    let mut goal_prompt = openai_client
        .get_completion(
            &ChatCompletionMessageParam::start_of_conversation(system_prompt, prompt),
            &ToolMap::new(),
        )
        .await
        .ok()
        .and_then(|v| v.first().cloned())
        .and_then(|v| v.content)
        .unwrap_or_else(|| format!("fulfill the tasks to reach this goal: {goal}"));
    let refined_prompt = goal_prompt.clone();
    debug_message(bot, username, &format!("refined prompt: {goal_prompt}"));
    for attempt in 0..20 {
        handle_agent_instruction(bot, state, &goal_prompt, username).await;
        let system_prompt = GOAL_VERIFIER.replace("$$GOAL$$", goal);
        let prompt = format!(
            "Using your available tools, determine if the agent successfully completed this goal. Start with TRUE or FALSE then explain your reasoning. Goal: {goal}"
        );
        let Ok(Some(verifier_response)) = openai_client
            .run_agent(
                ChatCompletionMessageParam::start_of_conversation(&system_prompt, prompt).to_vec(),
                &ToolMap::new()
                    .register_tool(ReadChatHistory::new(state))
                    .register_tool(ReadTabList::new(bot))
                    .register_tool(GetPartyInfo::new(bot, state))
                    .register_tool(GetCurrentPosition::new(bot))
                    .register_tool(GetInventory::new(bot))
                    .register_tool(NearbyBlockKinds::new(bot))
                    .register_tool(RunCommand::new(bot, state)),
            )
            .await
            .map(|v| v.0.content)
        else {
            debug_message(
                bot,
                username,
                "failed to verify if goal was reached, stopping.",
            );
            break;
        };
        debug_message(bot, username, &format!("verifier: {verifier_response}"));
        let goal_result: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let system_prompt = "You will be given a verifier's assessment. if the verifier says the agent fulfilled the goal then call the tool goal_completed. if not, call the tool goal_not_completed.";
        let prompt = format!(
            "Based on the verifier's assessment below, determine whether the agent fulfilled the goal. Call goal_completed or goal_not_completed accordingly.\n\nGoal: {goal}\n\nVerifier's assessment: {verifier_response}"
        );
        if openai_client
            .run_agent(
                ChatCompletionMessageParam::start_of_conversation(system_prompt, prompt).to_vec(),
                &ToolMap::new()
                    .register_tool(GoalCompleted(goal_result.clone()))
                    .register_tool(GoalNotCompleted(goal_result.clone())),
            )
            .await
            .is_err()
        {
            debug_message(bot, username, "failed to get verifier's sentiment.");
            break;
        }
        let completed = *goal_result.lock().await;
        debug_message(bot, username, &format!("sentiment result: {completed}"));

        if completed {
            debug_message(bot, username, "agent reached goal");
            break;
        }

        if attempt == 19 {
            debug_message(bot, username, "goal failed after 20 attempts, stopping.");
            break;
        }

        let system_prompt = GOAL_REFINER;
        let prompt = format!(
            "The previous agent failed this task. Analyze why based on the verifier's feedback, then write a corrective 2-3 sentence prompt for a new agent.\n\nPrevious task: {goal_prompt}\nVerifier feedback: {verifier_response}\n\nFocus on what went wrong and what the new agent must do differently. The final task that should be fulfilled:\n\n{refined_prompt}"
        );
        goal_prompt = openai_client
            .get_completion(
                &ChatCompletionMessageParam::start_of_conversation(system_prompt, prompt),
                &ToolMap::new(),
            )
            .await
            .ok()
            .and_then(|v| v.first().cloned())
            .and_then(|v| v.content)
            .unwrap_or_else(|| goal_prompt.clone());
        debug_message(bot, username, &format!("renewed prompt: {goal_prompt}"));
    }
    debug_message(bot, username, "finished goal loop...");
}

pub fn debug_message(bot: &Client, username: &str, message: &str) {
    let minceraft = format!("/msg {username} {message}")
        .chars()
        .take(100)
        .collect::<String>();
    bot.chat(minceraft);
    eprintln!("{message}");
}

static REQWEST_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);
async fn notify_roby_join(bot: &Client, state: &State) {
    let player_stats = get_stats("Roby98", bot, state).await;
    let mut content = String::from("Roby98 joined the server!\n**Bedwars Stats:**\n");
    write_player_stats_markdown(&player_stats, &mut content);
    if let Err(e) = send_discord_webhook(content).await {
        eprintln!("Failed sending roby webhook: {e}");
    }
}

async fn send_discord_webhook(markdown: String) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct MessageWebhook {
        content: String,
    }
    let body = MessageWebhook { content: markdown };
    let body = serde_json::to_string(&body).with_context(|| "failed to turn markdown into json")?;
    REQWEST_CLIENT.post("REDACTED").body(body).header("Content-Type", "application/json").send().await.map_err(|e| anyhow!("Reqwest: {e}"))?;
    Ok(())
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
    ) -> Result<Vec<ChatCompletionMessageParam>, ()>;
}

impl AiFunctionality for Client {
    async fn run_agent_loop(
        &self,
        ai_client: &OpenAIClient,
        system_prompt: &str,
        prompt: &str,
        username: &str,
        tools: ToolMap<'_>,
    ) -> Result<Vec<ChatCompletionMessageParam>, ()> {
        println!("running agent loop");
        let messages = vec![
            openai_client::ChatCompletionMessageParam::new_system(system_prompt),
            openai_client::ChatCompletionMessageParam::new_user(prompt),
        ];
        let res = ai_client.run_agent(messages, &tools).await.map_err(|e| {
            self.chat(format!("/msg {username} failed to run agent loop..."));
            println!("{e:#?}");
        });
        if let Ok((result, _)) = &res {
            eprintln!(
                "agent loop final response: {}",
                &result.content.as_ref().unwrap_or(&String::new())
            );
        }
        self.chat(format!("/msg {username} finished agent loop..."));
        Ok(res?.1)
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
