#![allow(clippy::cast_precision_loss)]

use azalea::{
    BlockPos, Client, Vec3,
    pathfinder::{PathfinderOpts, goals::BlockPosGoal},
    prelude::PathfinderClientExt,
};
use std::{fmt::Write, pin::Pin, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};
const ONE_POINT_EIGHT_MIN_Y: i32 = 0;

pub trait IntoPinBoxString<'a>: Sized + Send + ToString + 'a {
    fn into_pin_box_string(self) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move { self.to_string() })
    }
}

impl<'a, T: Sized + Send + ToString + 'a> IntoPinBoxString<'a> for T {}

use crate::{
    ENV_VARS, State,
    agent::tool::{ToolCallArgDescriptor, ToolCallFn},
    is_spam_message,
};

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

    fn get_args(&self) -> Vec<crate::agent::tool::ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "message",
            crate::agent::tool::ToolCallArgType::String,
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
        let mut response = String::new();
        let message = if message.chars().count() > 100 {
            response
                .push_str("Message was too long, truncated to 100 characters. past messages: \n\n");
            message.chars().take(100).collect::<String>()
        } else {
            response.push_str("Sent message. past messages: \n\n");
            message.clone()
        };
        let past_messages = self.1.messages.clone();
        self.0.chat(message);
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(8)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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

pub struct RespondToUser<'a, 'b, 'c>(&'a Client, &'b str, &'c State);

impl<'a, 'b, 'c> RespondToUser<'a, 'b, 'c> {
    pub fn new(client: &'a Client, user: &'b str, state: &'c State) -> Self {
        Self(client, user, state)
    }
}

impl ToolCallFn for RespondToUser<'_, '_, '_> {
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
            crate::agent::tool::ToolCallArgType::String,
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
        let mut response = String::new();
        let message = if message.chars().count() > 100 {
            response
                .push_str("Message was too long, truncated to 100 characters. past messages: \n\n");
            message.chars().take(100).collect::<String>()
        } else {
            response.push_str("Sent message. past messages: \n\n");
            message.clone()
        };
        let username = self.1.to_string();
        let past_messages = self.2.messages.clone();
        self.0.chat(format!("/msg {username} {message}"));
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
                crate::agent::tool::ToolCallArgType::Number,
                "how many past messages to load",
            ),
            ToolCallArgDescriptor::new(
                false,
                "verbose",
                crate::agent::tool::ToolCallArgType::Bool,
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
                .filter(|v| verbose || !is_spam_message(v))
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

                        let past_messages = message_state
                            .lock()
                            .await
                            .iter()
                            .map(ToString::to_string)
                            .filter(|v| !is_spam_message(v))
                            .take(15)
                            .collect::<Vec<String>>()
                            .iter()
                            .rev()
                            .fold(String::new(), |mut acc, s| {
                                _ = writeln!(&mut acc, "{s}");
                                acc
                            });
                        format!(
                            "slept for {num} seconds, past few chat messages:\n {past_messages}"
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
            crate::agent::tool::ToolCallArgType::Number,
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            if past_messages.contains("entrato nel party!") {
                "Successfully joined party".into()
            } else {
                format!("Failed joining party, chat history:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            crate::agent::tool::ToolCallArgType::String,
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
            crate::agent::tool::ToolCallArgType::String,
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
        let message_state = self.1.messages.clone();
        let mut response = String::new();
        let message = if message.chars().count() > 100 {
            response
                .push_str("Message was too long, truncated to 100 characters. past messages: \n\n");
            message.chars().take(100).collect::<String>()
        } else {
            response.push_str("Sent message. past messages: \n\n");
            message.clone()
        };
        let username = username.clone();
        self.0.chat(format!("/msg {username} {message}"));
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = message_state
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(10)
                .collect::<Vec<String>>()
                .iter()
                .rev()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            response.push_str(&past_messages);
            response
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::new(
                true,
                "username",
                crate::agent::tool::ToolCallArgType::String,
                "the user to send the direct message to",
            ),
            ToolCallArgDescriptor::new(
                true,
                "message",
                crate::agent::tool::ToolCallArgType::String,
                "the message to send to the user, cannot be longer than 100 characters",
            ),
        ]
    }
}

pub struct StopAgentLoop<'a, 'b>(Arc<Mutex<bool>>, &'a str, &'b Client);

impl<'a, 'b> StopAgentLoop<'a, 'b> {
    pub fn new(mutex: Arc<Mutex<bool>>, username: &'a str, client: &'b Client) -> Self {
        Self(mutex, username, client)
    }
}

impl ToolCallFn for StopAgentLoop<'_, '_> {
    fn get_name(&self) -> &'static str {
        "stop_agent_loop"
    }

    fn get_description(&self) -> &'static str {
        "stop running the agent loop meaning that you will not continue doing anything after calling this"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let should_stop = self.0.clone();
        let username = self.1;
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
        if number > 8 {
            return "the provided index has to be in range 0..=8".into_pin_box_string();
        }
        self.0
            .set_selected_hotbar_slot(number.try_into().unwrap_or_default());
        format!("selected hotbar slot {number}").into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "index",
            crate::agent::tool::ToolCallArgType::Number,
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
        let message = message.clone();
        let past_messages = self.1.messages.clone();
        let mut response = String::new();
        let message = if message.chars().count() > 100 {
            response
                .push_str("Message was too long, truncated to 100 characters. past messages: \n\n");
            message.chars().take(100).collect::<String>()
        } else {
            response.push_str("Sent message. past messages: \n\n");
            message
        };
        self.0.chat(format!("/pc {message}"));
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let chat_history = past_messages
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
                .fold(String::new(), |mut acc, s| {
                    _ = writeln!(&mut acc, "{s}");
                    acc
                });
            response.push_str(&chat_history);
            response
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "message",
            crate::agent::tool::ToolCallArgType::String,
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
            let past_messages = message_queue
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(6)
                .collect::<Vec<String>>()
                .iter()
                .rev()
                .fold(String::new(), |mut acc, message| {
                    _ = writeln!(&mut acc, "{message}");
                    acc
                });

            format!("requested party info, check chat for details: \n\n{past_messages}")
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
            crate::agent::tool::ToolCallArgType::String,
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(5)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
            crate::agent::tool::ToolCallArgType::String,
            "the username of the player to remove from friends",
        )]
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
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = past_messages
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(10)
                .collect::<Vec<String>>()
                .iter()
                .rev()
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                });
            format!(
                "Sent stats request, here is the chat history after the request:\n {past_messages}"
            )
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::new(
            true,
            "username",
            crate::agent::tool::ToolCallArgType::String,
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
                    let kind = block.as_registry_block().to_string();

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
                crate::agent::tool::ToolCallArgType::Number,
                "the maximum z radius a block can have from you to be read, defaults to 3",
            ),
            ToolCallArgDescriptor::new(
                false,
                "x_radius",
                crate::agent::tool::ToolCallArgType::Number,
                "the maximum x radius a block can have from you to be read, defaults to 5",
            ),
            ToolCallArgDescriptor::new(
                false,
                "y_radius",
                crate::agent::tool::ToolCallArgType::Number,
                "the maximum y radius a block can have from you to be read, defaults to 5",
            ),
            ToolCallArgDescriptor::new(
                false,
                "block_type",
                crate::agent::tool::ToolCallArgType::String,
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
                return "failed: path calculation timeout".into();
            }

            let start = Instant::now();

            let response = loop {
                let dist = bot.position().distance_to(Vec3 {
                    x: f64::from(x),
                    y: f64::from(y),
                    z: f64::from(z),
                });
                if dist <= 1.5 {
                    bot.stop_pathfinding();
                    break "success: reached goal".into();
                }

                if start.elapsed() > Duration::from_secs(10) {
                    break format!("failed: goal not reached in 10 seconds (distance {dist:.2})");
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
                crate::agent::tool::ToolCallArgType::Number,
                "the x coordinate of the position to pathfind to as signed integer",
            ),
            ToolCallArgDescriptor::new(
                true,
                "y",
                crate::agent::tool::ToolCallArgType::Number,
                "the y coordinate of the position to pathfind to as signed integer",
            ),
            ToolCallArgDescriptor::new(
                true,
                "z",
                crate::agent::tool::ToolCallArgType::Number,
                "the z coordinate of the position to pathfind to as signed integer",
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
                    let kind = block.as_registry_block().to_string();
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
                crate::agent::tool::ToolCallArgType::Number,
                "the maximum z radius a block can have from you to be read, defaults to 3",
            ),
            ToolCallArgDescriptor::new(
                false,
                "x_radius",
                crate::agent::tool::ToolCallArgType::Number,
                "the maximum x radius a block can have from you to be read, defaults to 5",
            ),
            ToolCallArgDescriptor::new(
                false,
                "y_radius",
                crate::agent::tool::ToolCallArgType::Number,
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
            crate::agent::tool::ToolCallArgType::String,
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
            crate::agent::tool::ToolCallArgType::String,
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(3)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
                crate::agent::tool::ToolCallArgType::String,
                "the command category to run",
            ),
            ToolCallArgDescriptor::new(
                false,
                "args",
                crate::agent::tool::ToolCallArgType::String,
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
            "pinfo" => {
                self.0.chat("/p info");
                "executed party info command."
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
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
                .take(8)
                .collect::<Vec<String>>()
                .iter()
                .rev()
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
            crate::agent::tool::ToolCallArgType::String,
            "quickcommand, choice of one of these choices:

pinfo: get party info
playtwo: join a duo game
playthree: join a trio game
stats: get your own stats
ping: get your current ping",
        )]
    }
}
