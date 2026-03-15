#![allow(clippy::cast_precision_loss)]

use azalea::{
    BlockPos, Client, Vec3,
    pathfinder::{PathfinderOpts, goals::BlockPosGoal},
    prelude::PathfinderClientExt,
};
use openai_client::{ToolCallArgDescriptor, ToolCallArgType, ToolCallFn, ToolMap};
use std::{fmt::Write, pin::Pin, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};

const ONE_POINT_EIGHT_MIN_Y: i32 = 0;

pub trait IntoPinBoxString<'a>: Sized + Send + ToString + 'a {
    fn into_pin_box_string(self) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move { self.to_string() })
    }
}

impl<'a, T: Sized + Send + ToString + 'a> IntoPinBoxString<'a> for T {}

use crate::{ENV_VARS, IsSpamMessage__PiperizeTrait, State, buffer::ChatFifoOps};

const SUBAGENT_PROMPT: &str = include_str!("./SUBAGENT.md");
pub struct SpawnSubAgent<'a> {
    subagent_tools: Arc<ToolMap<'a>>,
}

impl<'a> SpawnSubAgent<'a> {
    pub fn new(subagent_tools: Arc<ToolMap<'a>>) -> Self {
        Self { subagent_tools }
    }
}

impl ToolCallFn for SpawnSubAgent<'_> {
    fn get_name(&self) -> &'static str {
        "spawn_sub_agent"
    }

    fn get_description(&self) -> &'static str {
        "spawn an AI that can control the bot and interact with the outside world"
    }

    fn invoke<'b>(
        &'b self,
        args: &serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = String> + Send + 'b>> {
        let Some(serde_json::Value::String(task)) = args.get("task") else {
            return "Please provide a task".into_pin_box_string();
        };
        println!("Summoned Subagent with task: {task}");

        let task = task.clone();
        let tools = self.subagent_tools.clone();
        let agent_loop = async move {
            let Some(client) = &ENV_VARS.openai_client else {
                return "for some reason agent is not available right now".into();
            };
            // let available_tools_msg = tools.values().fold(String::new(), |mut acc, v| {
            //     _ = writeln!(&mut acc, "{}: {}", v.get_name(), v.get_description());
            //     acc
            // });
            client
                .run_agent(SUBAGENT_PROMPT, task, &tools)
                .await
                .unwrap_or(String::from("Subagent Run Failed"))
        };
        Box::pin(agent_loop)
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "task",
            ToolCallArgType::String,
            "the task that the agent should fulfill before stopping.",
        )]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct SendChatMessage<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> SendChatMessage<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for SendChatMessage<'_, '_> {
    fn get_name(&self) -> &'static str {
        "send_chat_message"
    }

    fn get_description(&self) -> &'static str {
        "send a message to global chat as the bot"
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "message",
            ToolCallArgType::String,
            "the message to send to global chat, cannot be more than 100 characters",
        )]
    }

    fn invoke(
        &self,
        args: &serde_json::Value,
    ) -> std::pin::Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(message)) = args.get("message") else {
            return "please provide a \"message\" field in the tool call".into_pin_box_string();
        };
        if message.chars().count() > 100 {
            return "Message exceeds 100 characters, please shorten it and try again..."
                .into_pin_box_string();
        }
        let mut response = String::new();
        let past_messages = self.1.messages.clone();
        self.0.chat(message);
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 8)
                .iter()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            response.push_str(&past_messages);
            response
        })
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(15000)
    }
}

pub struct RespondToUser(Arc<Client>, String, Arc<State>);

impl RespondToUser {
    pub fn new(client: Arc<Client>, username: String, state: Arc<State>) -> Self {
        Self(client, username, state)
    }
}

impl ToolCallFn for RespondToUser {
    fn get_name(&self) -> &'static str {
        "respond_to_user"
    }

    fn get_description(&self) -> &'static str {
        "respond to the user as bot ingame in private messages"
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "message",
            ToolCallArgType::String,
            "the message to send to the user",
        )]
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(message_val) = args.get("message") else {
            return "please provide a \"message\" field in the tool call".into_pin_box_string();
        };
        let serde_json::Value::String(message) = message_val else {
            return "message has to be of type string".into_pin_box_string();
        };
        if message.chars().count() > 100 {
            return "Message longer than 100 characters, please shorten it and try again..."
                .into_pin_box_string();
        }
        let mut response = String::new();
        let username = self.1.clone();
        let past_messages = self.2.messages.clone();
        self.0.chat(format!("/msg {username} {message}"));
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            response.push_str(&past_messages);
            response
        })
    }
}

pub struct ReadTabList<'a>(&'a Client);

impl<'a> ReadTabList<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for ReadTabList<'_> {
    fn get_name(&self) -> &'static str {
        "read_tab_list"
    }

    fn get_description(&self) -> &'static str {
        "read what players are in the current lobby, may contain rank and guild aswell"
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        self.0
            .tab_list()
            .values()
            .fold(String::new(), |mut acc, val| {
                if let Some(name) = &val.display_name {
                    _ = writeln!(&mut acc, "{name}");
                }
                acc
            })
            .into_pin_box_string()
    }
}

pub struct ReadChatHistory<'a>(&'a State);

impl<'a> ReadChatHistory<'a> {
    pub fn new(state: &'a State) -> Self {
        Self(state)
    }
}

impl ToolCallFn for ReadChatHistory<'_> {
    fn get_name(&self) -> &'static str {
        "read_chat_history"
    }

    fn get_description(&self) -> &'static str {
        "get the past messages from chat, do not request too many in order to not fill up context too quick"
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                false,
                "count",
                ToolCallArgType::Number,
                "how many past messages to load",
            ),
            ToolCallArgDescriptor::new(
                false,
                "verbose",
                ToolCallArgType::Bool,
                "whether to also show server announcements and join messages",
            ),
        ]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let num_messages = match args.get("count") {
            Some(serde_json::Value::Number(count)) => count.as_u64().unwrap_or(20),

            _ => 20,
        };
        let verbose = args
            .get("verbose")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let msg_q = self.0.messages.clone();
        Box::pin(async move {
            msg_q
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| verbose || !v.is_spam_message())
                .take(num_messages.try_into().unwrap_or(20))
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                })
        })
    }
}

pub struct Sleep<'a>(&'a State);

impl<'a> Sleep<'a> {
    pub fn new(state: &'a State) -> Self {
        Self(state)
    }
}

impl ToolCallFn for Sleep<'_> {
    fn get_name(&self) -> &'static str {
        "sleep"
    }

    fn get_description(&self) -> &'static str {
        "do nothing for a set amount of time"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        match args.get("time") {
            Some(serde_json::Value::Number(num)) => {
                if let Some(num) = num.as_u64() {
                    let message_state = self.0.messages.clone();
                    Box::pin(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(num)).await;
                        let past_messages_count: usize =
                            num.try_into().unwrap_or(25_usize).div_ceil(5);

                        let past_messages = message_state
                            .lock()
                            .await
                            .get_past_n_messages(false, past_messages_count)
                            .iter()
                            .fold(String::new(), |mut acc, s| {
                                _ = writeln!(&mut acc, "{s}");
                                acc
                            });
                        format!(
                            "slept for {num} seconds, past {past_messages_count} chat messages:\n {past_messages}"
                        )
                    })
                } else {
                    "timeout was invalid number, not sleeping".into_pin_box_string()
                }
            }
            _ => "No timeout provided or in the wrong format, not sleeping".into_pin_box_string(),
        }
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "time",
            ToolCallArgType::Number,
            "time to sleep in seconds as an unsigned integer",
        )]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct JoinParty<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> JoinParty<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for JoinParty<'_, '_> {
    fn get_name(&self) -> &'static str {
        "join_party"
    }

    fn get_description(&self) -> &'static str {
        "join the party of a player that has invited you or has a public party"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "did not provide correct arguments for party join".into_pin_box_string();
        };
        self.0.chat(format!("/p join {username}"));
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if past_messages.contains("entrato nel party!") {
                "Successfully joined party".into()
            } else if past_messages.contains("Non hai richieste da parte di questo giocatore.") {
                "You don't have a party request from that player.".into()
            } else {
                format!("Failed joining party, chat history:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the name of the user whose party you want to join",
        )]
    }
}
pub struct InviteParty<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> InviteParty<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for InviteParty<'_, '_> {
    fn get_name(&self) -> &'static str {
        "invite_party"
    }

    fn get_description(&self) -> &'static str {
        "invite a player to your party (this will create a party if you're not already in one)"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "did not provide correct arguments for party invite".into_pin_box_string();
        };
        self.0.chat(format!("/p invite {username}"));
        let past_messages = self.1.messages.clone();
        let username = username.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if past_messages.contains("Hai invitato") && past_messages.contains("nel party!") {
                format!("Successfully invited {username} to a party.")
            } else if past_messages.contains("Questo giocatore risulta offline...") {
                "This player either is offline or does not exist.".into()
            } else if past_messages
                .contains("Hai già inviato una richiesta di party a questo giocatore!")
            {
                "You already invited this player to the party.".into()
            } else if past_messages.contains("Questo giocatore risulta essere in un altro party!") {
                "This player is already in another party.".into()
            } else {
                format!("Failed inviting {username} to the party. chat history:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the name of the user who you want to invite to a party",
        )]
    }
}

pub struct SendDirectMessage<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> SendDirectMessage<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for SendDirectMessage<'_, '_> {
    fn get_name(&self) -> &'static str {
        "send_direct_message"
    }

    fn get_description(&self) -> &'static str {
        "send a direct message to a player"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username to send the dm to".into_pin_box_string();
        };
        let Some(serde_json::Value::String(message)) = args.get("message") else {
            return "please provide a message to send to the user".into_pin_box_string();
        };
        if message.chars().count() > 100 {
            return "Message exceeds 100 characters, please shorten it and try again..."
                .into_pin_box_string();
        }
        let message_state = self.1.messages.clone();
        let username = username.clone();
        self.0.chat(format!("/msg {username} {message}"));
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let messages: Vec<String> = message_state
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !v.is_spam_message())
                .take(10)
                .collect();

            if messages
                .iter()
                .rev()
                .take(3)
                .any(|m| m.contains("Questo giocatore non risulta online!"))
            {
                return format!("failed: player `{username}` is not online or does not exist");
            }

            let past_messages = messages.iter().rev().fold(String::new(), |mut acc, s| {
                _ = writeln!(&mut acc, "{s}");
                acc
            });

            format!("Sent direct message, past 10 chat messages: {past_messages}",)
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                true,
                "username",
                ToolCallArgType::String,
                "the user to send the direct message to",
            ),
            ToolCallArgDescriptor::new(
                true,
                "message",
                ToolCallArgType::String,
                "the message to send to the user, cannot be longer than 100 characters",
            ),
        ]
    }
}

pub struct StopAgentLoop<'a>(Arc<Mutex<bool>>, String, &'a Client);

impl<'a> StopAgentLoop<'a> {
    pub fn new(mutex: Arc<Mutex<bool>>, username: String, client: &'a Client) -> Self {
        Self(mutex, username, client)
    }
}

impl ToolCallFn for StopAgentLoop<'_> {
    fn get_name(&self) -> &'static str {
        "stop_agent_loop"
    }

    fn get_description(&self) -> &'static str {
        "stop running the agent loop meaning that you will not continue doing anything after calling this"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let should_stop = self.0.clone();
        let username = self.1.clone();
        self.2
            .chat(format!("/msg {username} Stopping Agent Loop..."));
        Box::pin(async move {
            let mut should_stop = should_stop.lock().await;
            *should_stop = true;
            "stopped the agent loop".into()
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct GetInventory<'a>(&'a Client);

impl<'a> GetInventory<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for GetInventory<'_> {
    fn get_name(&self) -> &'static str {
        "get_inventory"
    }

    fn get_description(&self) -> &'static str {
        "get your current inventory, counts top down, left to right meaning the hotbar starts at slot 36"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        if self.0.exists()
            && let Some(inventory) = self.0.open_inventory()
            && let Some(slots) = inventory.slots()
        {
            slots
                .iter()
                .enumerate()
                .fold(
                    String::from("Current inventory:\n"),
                    |mut acc, (idx, slot)| {
                        let kind = slot.kind().to_string();
                        _ = writeln!(
                            &mut acc,
                            "[index {}]: {}",
                            idx,
                            kind.split_once(':').map_or(kind.as_str(), |v| v.1)
                        );
                        acc
                    },
                )
                .into_pin_box_string()
        } else {
            "couldn't get inventory".into_pin_box_string()
        }
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct SelectHotbarSlot<'a>(&'a Client);

impl<'a> SelectHotbarSlot<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for SelectHotbarSlot<'_> {
    fn get_name(&self) -> &'static str {
        "select_hotbar_slot"
    }

    fn get_description(&self) -> &'static str {
        "select which item to hold in your main hand"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::Number(index)) = args.get("index") else {
            return "please provide argument index of type number".into_pin_box_string();
        };
        let Some(number) = index.as_u64() else {
            return "the provided index has to be an unsigned integer".into_pin_box_string();
        };
        let number = number.rem_euclid(9);
        self.0
            .set_selected_hotbar_slot(number.try_into().unwrap_or_default());
        format!("selected hotbar slot {number}").into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "index",
            ToolCallArgType::Number,
            "the hotbar index to select as current main hand item",
        )]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct SendPartyMessage<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> SendPartyMessage<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for SendPartyMessage<'_, '_> {
    fn get_name(&self) -> &'static str {
        "send_party_message"
    }

    fn get_description(&self) -> &'static str {
        "send a message to party chat only"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(message)) = args.get("message") else {
            return "please provide a message to send to the party".into_pin_box_string();
        };
        if message.chars().count() > 100 {
            return "Message exceeds 100 characters, please shorten it and try again..."
                .into_pin_box_string();
        }
        let past_messages = self.1.messages.clone();
        let mut response = String::new();
        self.0.chat(format!("/pc {message}"));
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let chat_history = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            if chat_history.contains("Non sei in nessun party!") {
                return "You are not in a party!".into();
            }
            response.push_str(&chat_history);
            response
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "message",
            ToolCallArgType::String,
            "the message to send to party chat, max 100 characters",
        )]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(1000)
    }
}

pub struct LeaveParty<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> LeaveParty<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for LeaveParty<'_, '_> {
    fn get_name(&self) -> &'static str {
        "leave_party"
    }

    fn get_description(&self) -> &'static str {
        "leave your current party"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        self.0.chat("/p leave");
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if past_messages.contains("uscito") || past_messages.contains("stato sciolto!") {
                "Successfully left the party".into()
            } else {
                "Failed leaving the party as you were in no party.".into()
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }
}

pub struct PartyInfo {
    owner: String,
    members: Vec<String>,
    opened: bool,
    muted: bool,
}
impl PartyInfo {
    #[must_use]
    fn new(owner: String) -> Self {
        Self {
            owner,
            members: Vec::new(),
            opened: false,
            muted: false,
        }
    }
    #[allow(unused)]
    #[must_use]
    fn set_owner(mut self, s: impl Into<String>) -> Self {
        self.owner = s.into();
        self
    }
    #[must_use]
    fn set_opened(mut self) -> Self {
        self.opened = true;
        self
    }
    #[must_use]
    fn set_muted(mut self) -> Self {
        self.muted = true;
        self
    }
    #[must_use]
    fn set_members(mut self, u: impl Iterator<Item = String>) -> Self {
        for user in u {
            self.members.push(user);
        }
        self
    }
    pub fn party_info_to_llm_text(&self) -> String {
        let info = self;
        let members = if info.members.is_empty() {
            "no members".to_string()
        } else {
            format!("members {}", info.members.join(", "))
        };

        let opened = if info.opened { "open" } else { "closed" };
        let muted = if info.muted { "muted" } else { "not muted" };

        format!(
            "{}'s party has {}, it is {} and currently {}.",
            info.owner, members, opened, muted
        )
    }
    pub fn parse_party_info(input: &str) -> Option<PartyInfo> {
        let input = input.trim();
        let mut owner = None;
        let mut members: Vec<String> = Vec::new();
        let mut opened = false;
        let mut muted = false;

        for line in input.lines().map(str::trim) {
            if let Some(v) = line.strip_prefix("Party di ") {
                owner = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("Membri:") {
                let v = v.trim();

                if v != "Nessuno" && !v.is_empty() {
                    members = v.split(',').map(|m| m.trim().to_string()).collect();
                }
            } else if let Some(v) = line.strip_prefix("Aperto:") {
                let v = v.trim();
                opened = matches!(v, "Sì" | "Si" | "Yes" | "true");
            } else if let Some(v) = line.strip_prefix("Mutato:") {
                let v = v.trim();
                muted = matches!(v, "Sì" | "Si" | "Yes" | "true");
            }
        }

        let mut info = PartyInfo::new(owner?);

        if opened {
            info = info.set_opened();
        }

        if muted {
            info = info.set_muted();
        }

        if !members.is_empty() {
            info = info.set_members(members.into_iter());
        }

        Some(info)
    }
}

pub struct GetPartyInfo<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> GetPartyInfo<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for GetPartyInfo<'_, '_> {
    fn get_name(&self) -> &'static str {
        "get_party_info"
    }

    fn get_description(&self) -> &'static str {
        "get information about your current party"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        self.0.chat("/p info");
        let message_queue = self.1.messages.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let mut p_info: Option<PartyInfo> = None;
            let past_messages = message_queue
                .lock()
                .await
                .get_past_n_messages(false, 6)
                .iter()
                .fold(String::new(), |mut acc, v| {
                    if v.contains("Party di")
                        && v.contains("Membri:")
                        && v.contains("Aperto:")
                        && v.contains("Mutato:")
                    {
                        p_info = PartyInfo::parse_party_info(v);
                    }
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if let Some(party_info) = p_info {
                party_info.party_info_to_llm_text()
            } else if past_messages.contains("Nessun party trovato!") {
                "You are not in a party.".into()
            } else {
                format!("requested party info, check chat for details: \n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }
}

pub struct AddFriend<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> AddFriend<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for AddFriend<'_, '_> {
    fn get_name(&self) -> &'static str {
        "add_friend"
    }

    fn get_description(&self) -> &'static str {
        "send a friend request to a player"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username to add as friend".into_pin_box_string();
        };
        self.0.chat(format!("/f add {username}"));
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if past_messages.contains("Hai inviato una richiesta di amicizia") {
                "Successfully sent friend request".into()
            } else if past_messages.contains("Il giocatore specificato non è online.") {
                "The player is not online.".into()
            } else if past_messages.contains("Hai già inviato una richiesta di amicizia a") {
                "You already sent a friend request to this player".into()
            } else {
                format!("Failed sending friend request. chat history:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the username of the player to add as friend",
        )]
    }
}

pub struct RemoveFriend<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> RemoveFriend<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for RemoveFriend<'_, '_> {
    fn get_name(&self) -> &'static str {
        "remove_friend"
    }

    fn get_description(&self) -> &'static str {
        "remove a player from your friends list"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username to remove from friends".into_pin_box_string();
        };
        self.0.chat(format!("/f remove {username}"));
        let past_messages = self.1.messages.clone();
        let username = username.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if past_messages.contains("Hai rimosso") && past_messages.contains("dai tuoi amici.") {
                format!("Successfully removed {username} from your friends list.")
            } else if past_messages.contains("Non sei amico di") {
                format!("You are not friends with {username}.")
            } else {
                format!("Failed Removing friend. chat history:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the username of the player to remove from friends",
        )]
    }
}

#[derive(Debug, Default)]
pub struct PlayerStats {
    pub level: Option<u32>,
    pub xp: Option<u32>,
    pub coins: Option<u64>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub kd: Option<f32>,
    pub final_kills: Option<u32>,
    pub final_deaths: Option<u32>,
    pub fk_fd: Option<f32>,
    pub beds_broken: Option<u32>,
    pub wins: Option<u32>,
    pub losses: Option<u32>,
    pub winrate: Option<f32>,
    pub division: Option<String>,
    pub division_xp: Option<u32>,
    pub winstreak: Option<u32>,
    pub max_winstreak: Option<u32>,
}

impl PlayerStats {
    pub fn parse_from_chat<I>(lines: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut stats = PlayerStats::default();

        for line in lines {
            let line = line.as_ref().trim();

            if let Some(v) = line.strip_prefix("Livello: ") {
                stats.level = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("XP: ") {
                stats.xp = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Coins: ") {
                stats.coins = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Uccisioni: ") {
                stats.kills = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Morti: ") {
                stats.deaths = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("K/D: ") {
                stats.kd = v.parse().ok();
            } else if let Some(v) = line.strip_prefix("Uccisioni finali: ") {
                stats.final_kills = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Morti finali: ") {
                stats.final_deaths = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("FK/FD: ") {
                stats.fk_fd = v.parse().ok();
            } else if let Some(v) = line.strip_prefix("Letti distrutti: ") {
                stats.beds_broken = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Partite vinte: ") {
                let parts: Vec<_> = v.split_whitespace().collect();
                if let Some(win_loss) = parts.first() {
                    let mut wl = win_loss.split('/');
                    stats.wins = wl.next().and_then(|v| v.replace(',', "").parse().ok());
                    stats.losses = wl.next().and_then(|v| v.replace(',', "").parse().ok());
                }

                if let Some(rate) = parts.get(1) {
                    stats.winrate = rate
                        .trim_matches(|c| c == '(' || c == ')' || c == '%')
                        .parse()
                        .ok();
                }
            } else if let Some(v) = line.strip_prefix("Divisione: ") {
                if let Some((name, _)) = v.split_once('(') {
                    stats.division = Some(name.trim().to_string());
                }
            } else if let Some(v) = line.strip_prefix("XP Divisione: ") {
                stats.division_xp = v.replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Winstreak/Max winstreak: ") {
                let mut parts = v.split('/');
                stats.winstreak = parts.next().and_then(|v| v.trim().parse().ok());
                stats.max_winstreak = parts.next().and_then(|v| v.trim().parse().ok());
            }
        }

        stats
    }
    pub fn player_stats_to_llm_text(&self) -> String {
        let mut parts = Vec::new();

        if let Some(level) = self.level {
            parts.push(format!("level {level}"));
        }

        if let Some(coins) = self.coins {
            parts.push(format!("{coins} coins"));
        }

        if let (Some(k), Some(d)) = (self.kills, self.deaths) {
            if let Some(kd) = self.kd {
                parts.push(format!("{k} kills and {d} deaths (K/D {kd:.2})"));
            } else {
                parts.push(format!("{k} kills and {d} deaths"));
            }
        }

        if let (Some(fk), Some(fd)) = (self.final_kills, self.final_deaths) {
            if let Some(r) = self.fk_fd {
                parts.push(format!(
                    "{fk} final kills and {fd} final deaths (FK/FD {r:.2})"
                ));
            } else {
                parts.push(format!("{fk} final kills and {fd} final deaths"));
            }
        }

        if let Some(beds) = self.beds_broken {
            parts.push(format!("{beds} beds broken"));
        }

        if let (Some(w), Some(l)) = (self.wins, self.losses) {
            if let Some(rate) = self.winrate {
                parts.push(format!("{w} wins and {l} losses ({rate:.2}% winrate)"));
            } else {
                parts.push(format!("{w} wins and {l} losses"));
            }
        }

        if let Some(div) = &self.division {
            if let Some(xp) = self.division_xp {
                parts.push(format!("division {div} with {xp} division XP"));
            } else {
                parts.push(format!("division {div}"));
            }
        }

        if let Some(ws) = self.winstreak {
            if let Some(max) = self.max_winstreak {
                parts.push(format!("current winstreak {ws} (max {max})"));
            } else {
                parts.push(format!("winstreak {ws}"));
            }
        }

        parts.join(", ")
    }
}

pub struct GetStats<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> GetStats<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for GetStats<'_, '_> {
    fn get_name(&self) -> &'static str {
        "get_stats"
    }

    fn get_description(&self) -> &'static str {
        "get the bedwars stats of a player"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username string for whose stats to get".into_pin_box_string();
        };
        self.0.chat(format!("/stats {username}"));
        let past_messages = self.1.messages.clone();
        let username = username.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            let past_messages = past_messages.lock().await;
            let past_messages = past_messages
                .iter()
                .filter(|v| !v.to_string().is_spam_message())
                .take(25);
            let stats = PlayerStats::parse_from_chat(past_messages.map(ToString::to_string));
            format!("{username}'s stats: {}", stats.player_stats_to_llm_text())
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the username of the player whose stats to get",
        )]
    }
}

pub struct GetNearbyBlocks<'a>(&'a Client);

impl<'a> GetNearbyBlocks<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for GetNearbyBlocks<'_> {
    fn get_name(&self) -> &'static str {
        "get_nearby_blocks"
    }

    fn get_description(&self) -> &'static str {
        "get the blocks that are near you"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let radius = |name, default| {
            args.get(name)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(default)
        };

        let z_radius = radius("z_radius", 3);
        let x_radius = radius("x_radius", 5);
        let y_radius = radius("y_radius", 5);

        let requested_block = args
            .get("block_type")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(|v| format!("minecraft:{v}"));

        let bot_pos = self.0.position();
        let mut response = match requested_block {
            Some(ref block_type) => {
                let block_type = block_type
                    .split_once(':')
                    .map_or(block_type.as_str(), |v| v.1);
                format!(
                    "Showing nearby blocks of type {block_type} in format:\n[x,y,z]: Block\nYour Current Position [{:.0},{:.0},{:.0}]:\n\n",
                    bot_pos.x, bot_pos.y, bot_pos.z
                )
            }
            None => format!(
                "Showing all nearby blocks in format:\n[x,y,z]: Block\nYour Current Position [{:.0},{:.0},{:.0}]:\n\n",
                bot_pos.x, bot_pos.y, bot_pos.z
            ),
        };

        let world = self.0.partial_world().clone();
        let chunks = &world.read().chunks;

        let Some(chunks) = chunks.limited_get(&self.0.position().into()) else {
            return "failed to get nearby chunks".into_pin_box_string();
        };

        let chunks = chunks.read();

        for x in -x_radius..x_radius {
            for y in -y_radius..y_radius {
                for z in -z_radius..z_radius {
                    let pos = Vec3::new(
                        bot_pos.x + x as f64,
                        bot_pos.y + y as f64,
                        bot_pos.z + z as f64,
                    );

                    let Some(block) = chunks.get_block_state(&pos.into(), ONE_POINT_EIGHT_MIN_Y)
                    else {
                        continue;
                    };

                    if block.is_air() {
                        continue;
                    }

                    let block = Box::<dyn azalea::block::BlockTrait>::from(block);
                    let kind = block.as_block_kind().to_string();

                    if requested_block.as_ref().is_some_and(|r| r != &kind) {
                        continue;
                    }

                    let _ = writeln!(
                        &mut response,
                        "[{:.0},{:.0},{:.0}]: {}",
                        pos.x,
                        pos.y,
                        pos.z,
                        kind.split_once(':').map_or(kind.as_str(), |v| v.1)
                    );
                }
            }
        }

        response.into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                false,
                "z_radius",
                ToolCallArgType::Number,
                "the maximum z radius a block can have from you to be read, defaults to 3",
            ),
            ToolCallArgDescriptor::new(
                false,
                "x_radius",
                ToolCallArgType::Number,
                "the maximum x radius a block can have from you to be read, defaults to 5",
            ),
            ToolCallArgDescriptor::new(
                false,
                "y_radius",
                ToolCallArgType::Number,
                "the maximum y radius a block can have from you to be read, defaults to 5",
            ),
            ToolCallArgDescriptor::new(
                false,
                "block_type",
                ToolCallArgType::String,
                "block type in lower snake case. if none is provided will show all blocks in the radius (excluding air). example: oak_log",
            ),
        ]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct PathFind<'a>(&'a Client);

impl<'a> PathFind<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for PathFind<'_> {
    fn get_name(&self) -> &'static str {
        "path_find"
    }

    fn get_description(&self) -> &'static str {
        "path find to a certain position, make sure that it is reachable, stairs are not traversible"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let arg_to_i32 = |arg: &serde_json::Value, name: &str| {
            arg.get(name)
                .and_then(serde_json::Value::as_i64)
                .and_then(|v| TryInto::<i32>::try_into(v).ok())
        };

        let Some(x) = arg_to_i32(args, "x") else {
            return "Please provie a valid x position in the arguments".into_pin_box_string();
        };
        let Some(y) = arg_to_i32(args, "y") else {
            return "Please provie a valid y position in the arguments".into_pin_box_string();
        };
        let Some(z) = arg_to_i32(args, "z") else {
            return "Please provie a valid z position in the arguments".into_pin_box_string();
        };
        let time_to_reach_goal = args
            .get("time_to_reach_goal")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(10);
        let bot = self.0.clone();
        Box::pin(async move {
            let goto_bot = bot.clone();
            let goto_thread = tokio::spawn(async move {
                goto_bot
                    .goto_with_opts(
                        BlockPosGoal(BlockPos::new(x, y, z)),
                        PathfinderOpts::new().allow_mining(false).max_timeout(
                            azalea::pathfinder::astar::PathfinderTimeout::Time(
                                Duration::from_millis(1000),
                            ),
                        ),
                    )
                    .await;
            });

            if tokio::time::timeout(Duration::from_millis(1500), async {
                while bot.is_calculating_path() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .is_err()
            {
                goto_thread.abort();
                return format!("Failed going to [{x},{y},{z}] due to timeout.");
            }

            let start = Instant::now();

            let response = loop {
                let bot_position = bot.position();
                let dist = bot_position.distance_to(Vec3 {
                    x: f64::from(x),
                    y: f64::from(y),
                    z: f64::from(z),
                });

                if dist <= 1.5 {
                    bot.stop_pathfinding();
                    break format!("Reached goal near [{x},{y},{z}] (distance {dist:.2})");
                }

                if goto_thread.is_finished() {
                    break format!(
                        "Pathfinder stopped before reaching [{x},{y},{z}] (distance {dist:.2}, current position: [{:.0},{:.0},{:.0}])",
                        bot_position.x, bot_position.y, bot_position.z
                    );
                }

                if start.elapsed() > Duration::from_secs(time_to_reach_goal) {
                    bot.stop_pathfinding();
                    break format!(
                        "Timeout reaching [{x},{y},{z}] after {time_to_reach_goal}s (distance {dist:.2}, current position: [{:.0},{:.0},{:.0}])",
                        bot_position.x, bot_position.y, bot_position.z
                    );
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            bot.stop_pathfinding();
            goto_thread.abort();
            response
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                true,
                "x",
                ToolCallArgType::Number,
                "the x coordinate of the position to pathfind to as signed integer",
            ),
            ToolCallArgDescriptor::new(
                true,
                "y",
                ToolCallArgType::Number,
                "the y coordinate of the position to pathfind to as signed integer",
            ),
            ToolCallArgDescriptor::new(
                true,
                "z",
                ToolCallArgType::Number,
                "the z coordinate of the position to pathfind to as signed integer",
            ),
            ToolCallArgDescriptor::new(
                true,
                "time_to_reach_goal",
                ToolCallArgType::Number,
                "The estimated time in seconds it should take you to reach that goal.",
            ),
        ]
    }
    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct GetCurrentPosition<'a>(&'a Client);

impl<'a> GetCurrentPosition<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for GetCurrentPosition<'_> {
    fn get_name(&self) -> &'static str {
        "get_current_position"
    }

    fn get_description(&self) -> &'static str {
        "get your current position"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        format!("current position: {}", self.0.position()).into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }
}

pub struct NearbyBlockKinds<'a>(&'a Client);

impl<'a> NearbyBlockKinds<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for NearbyBlockKinds<'_> {
    fn get_name(&self) -> &'static str {
        "nearby_block_kinds"
    }

    fn get_description(&self) -> &'static str {
        "get the block names as string in a set radius around you"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let radius = |name, default| {
            args.get(name)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(default)
        };

        let z_radius = radius("z_radius", 3);
        let x_radius = radius("x_radius", 5);
        let y_radius = radius("y_radius", 5);
        let world = self.0.partial_world().clone();
        let chunks = &world.read().chunks;
        let bot_pos = self.0.position();

        let Some(chunks) = chunks.limited_get(&self.0.position().into()) else {
            return "failed to get nearby chunks".into_pin_box_string();
        };

        let chunks = chunks.read();
        let mut found_blocks = String::from("Found block kinds:\n");

        for x in -x_radius..x_radius {
            for y in -y_radius..y_radius {
                for z in -z_radius..z_radius {
                    let pos = Vec3::new(
                        bot_pos.x + x as f64,
                        bot_pos.y + y as f64,
                        bot_pos.z + z as f64,
                    );

                    let Some(block) = chunks.get_block_state(&pos.into(), ONE_POINT_EIGHT_MIN_Y)
                    else {
                        continue;
                    };

                    if block.is_air() {
                        continue;
                    }

                    let block = Box::<dyn azalea::block::BlockTrait>::from(block);
                    let kind = block.as_block_kind().to_string();
                    let kind = kind.split_once(':').map_or(kind.as_str(), |v| v.1);
                    if !found_blocks.contains(kind) {
                        _ = writeln!(&mut found_blocks, "{kind}");
                    }
                }
            }
        }
        found_blocks.into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                false,
                "z_radius",
                ToolCallArgType::Number,
                "the maximum z radius a block can have from you to be read, defaults to 3",
            ),
            ToolCallArgDescriptor::new(
                false,
                "x_radius",
                ToolCallArgType::Number,
                "the maximum x radius a block can have from you to be read, defaults to 5",
            ),
            ToolCallArgDescriptor::new(
                false,
                "y_radius",
                ToolCallArgType::Number,
                "the maximum y radius a block can have from you to be read, defaults to 5",
            ),
        ]
    }
    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct DeclineFriendRequest<'a>(&'a Client);

impl<'a> DeclineFriendRequest<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for DeclineFriendRequest<'_> {
    fn get_name(&self) -> &'static str {
        "decline_friend_request"
    }

    fn get_description(&self) -> &'static str {
        "decline a friend request that you received"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "Please provide a username whose friend request to decline"
                .into_pin_box_string();
        };
        self.0.chat(format!("/f deny {username}"));
        format!("declined friend request by {username}").into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the username whose friend request should be declined",
        )]
    }
}
pub struct AcceptFriendRequest<'a>(&'a Client);

impl<'a> AcceptFriendRequest<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for AcceptFriendRequest<'_> {
    fn get_name(&self) -> &'static str {
        "accept_friend_request"
    }

    fn get_description(&self) -> &'static str {
        "accept a friend request that you received"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "Please provide a username whose friend request to accept"
                .into_pin_box_string();
        };
        self.0.chat(format!("/f accept {username}"));
        format!("accepted friend request by {username}").into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "the username whose friend request should be accepted",
        )]
    }
}

pub struct RunCommand<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> RunCommand<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for RunCommand<'_, '_> {
    fn get_name(&self) -> &'static str {
        "run_command"
    }

    fn get_description(&self) -> &'static str {
        "run any command"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(command)) = args.get("command") else {
            return "please provide a command".into_pin_box_string();
        };
        let arg = args
            .get("args")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        self.0.chat(format!("/{command} {arg}"));
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 3)
                .iter()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            if past_messages.contains("Comando sconosciuto.") {
                "Uknown Command".into()
            } else {
                format!("Ran command, past chat output:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                true,
                "command",
                ToolCallArgType::String,
                "the command category to run",
            ),
            ToolCallArgDescriptor::new(
                false,
                "args",
                ToolCallArgType::String,
                "the arguments to provide for additional configuration of the main command",
            ),
        ]
    }
}

pub struct QuickCommand<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> QuickCommand<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for QuickCommand<'_, '_> {
    fn get_name(&self) -> &'static str {
        "quick_command"
    }

    fn get_description(&self) -> &'static str {
        "run quick commands to get your own stats, get party info, join a game, etc..."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(command)) = args.get("command") else {
            return "please provide a command".into_pin_box_string();
        };
        let exc_comm = match command.trim() {
            "spawn" => {
                self.0.chat("/spawn");
                "teleported to spawn."
            }
            "playtwo" => {
                self.0.chat("/play two");
                "executed join duo command."
            }
            "playthree" => {
                self.0.chat("/play three");
                "executed join trio command."
            }
            "stats" => {
                self.0.chat(format!("/stats {}", ENV_VARS.username));
                "executed stats command."
            }
            "ping" => {
                self.0.chat("/ping");
                "executed ping command."
            }
            _ => return format!("command \"{command}\" does not exist").into_pin_box_string(),
        };
        let past_messages = self.1.messages.clone();
        let mut response = String::from(exc_comm);
        response.push_str("past messages:\n\n");

        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 8)
                .iter()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            response.push_str(&past_messages);
            response
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "command",
            ToolCallArgType::String,
            "quickcommand, choice of one of these choices:

spawn: teleport to the lobby spawn location
playtwo: join a duo game
playthree: join a trio game
stats: get your own stats
ping: get your current ping",
        )]
    }
}

pub struct JoinGame<'a, 'b>(&'a Client, &'b State);

impl<'a, 'b> JoinGame<'a, 'b> {
    pub fn new(client: &'a Client, state: &'b State) -> Self {
        Self(client, state)
    }
}

impl ToolCallFn for JoinGame<'_, '_> {
    fn get_name(&self) -> &'static str {
        "join_game"
    }

    fn get_description(&self) -> &'static str {
        "join someone's private game"
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "Please provide the username of the player whose game you want to join."
                .into_pin_box_string();
        };

        let past_messages = self.1.messages.clone();
        self.0.chat(format!("/join {username}"));
        Box::pin(async move {
            let past_messages = past_messages
                .lock()
                .await
                .get_past_n_messages(false, 5)
                .iter()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            if past_messages.contains("Nessun server privato trovato per quel giocatore.") {
                "This player is not hosting a private game".into()
            } else {
                "Successfully joined private game.".into()
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            ToolCallArgType::String,
            "whose private game to join",
        )]
    }
}
