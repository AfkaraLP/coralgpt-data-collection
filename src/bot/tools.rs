use azalea::Client;
use std::{fmt::Write, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

pub trait IntoPinBoxString<'a>: Sized + Send + ToString + 'a {
    fn into_pin_box_string(self) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move { self.to_string() })
    }
}

impl<'a, T: Sized + Send + ToString + 'a> IntoPinBoxString<'a> for T {}

use crate::{
    State,
    agent::tool::{ToolCallArgDescriptor, ToolCallFn},
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
        let Some(message_val) = args.get("message") else {
            return "please provide a \"message\" field in the tool call".into_pin_box_string();
        };
        let serde_json::Value::String(message) = message_val else {
            return "message has to be of type string".into_pin_box_string();
        };
        let past_messages = self.1.messages.clone();
        let bot = self.0.clone();
        let response_message = message.clone();
        Box::pin(async move {
            if response_message.chars().count() > 100 {
                let response_message = response_message.chars().take(100).collect::<String>();
                bot.chat(response_message);
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let past_messages =
                    past_messages
                        .lock()
                        .await
                        .iter()
                        .take(10)
                        .fold(String::new(), |mut acc, s| {
                            _ = writeln!(&mut acc, "{s}");
                            acc
                        });
                return format!(
                    "# Truncated message as it was too long, here is chat history with sent message: \n{past_messages}",
                );
            }
            bot.chat(response_message);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let past_messages =
                past_messages
                    .lock()
                    .await
                    .iter()
                    .take(10)
                    .fold(String::new(), |mut acc, s| {
                        _ = writeln!(&mut acc, "{s}");
                        acc
                    });
            format!(
                "# Successfully sent message, here is chat history with sent message: \n{past_messages}",
            )
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
        let message = message.clone();
        let bot = self.0.clone();
        let username = self.1.to_string();
        let past_messages = self.2.messages.clone();
        Box::pin(async move {
            if message.chars().count() > 100 {
                let message = message.chars().take(100).collect::<String>();
                bot.chat(format!("/msg {username} {message}"));
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let past_messages =
                    past_messages
                        .lock()
                        .await
                        .iter()
                        .take(10)
                        .fold(String::new(), |mut acc, s| {
                            _ = writeln!(&mut acc, "{s}");
                            acc
                        });
                return format!(
                    "message too long, truncating to 100 chars and sending anyway, new chat: \n\n{past_messages}",
                );
            }
            bot.chat(format!("/msg {username} {message}"));
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let past_messages =
                past_messages
                    .lock()
                    .await
                    .iter()
                    .take(10)
                    .fold(String::new(), |mut acc, s| {
                        _ = writeln!(&mut acc, "{s}");
                        acc
                    });
            format!("successfully sent message, new chat: \n\n{past_messages}")
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
        vec![ToolCallArgDescriptor::new(
            false,
            "count",
            crate::agent::tool::ToolCallArgType::Number,
            "how many past messages to load",
        )]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let num_messages = match args.get("count") {
            Some(serde_json::Value::Number(count)) => count.as_u64().unwrap_or(20),

            _ => 20,
        };
        let msg_q = self.0.messages.clone();
        Box::pin(async move {
            msg_q
                .lock()
                .await
                .iter()
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
                        tokio::time::sleep(std::time::Duration::from_millis(num)).await;
                        let past_messages = message_state.lock().await.iter().take(20).fold(
                            String::new(),
                            |mut acc, s| {
                                _ = writeln!(&mut acc, "{s}");
                                acc
                            },
                        );
                        format!(
                            "# slept for {num} milliseconds, past 20 chat messages:\n {past_messages}"
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
            "time to sleep in milliseconds as an unsigned integer",
        )]
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(0)
    }
}

pub struct JoinParty<'a>(&'a Client);

impl<'a> JoinParty<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for JoinParty<'_> {
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
        "successfully sent join party command".into_pin_box_string()
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
pub struct InviteParty<'a>(&'a Client);

impl<'a> InviteParty<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for InviteParty<'_> {
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
        "successfully sent invite party command".into_pin_box_string()
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
        let bot = self.0.clone();
        let message_state = self.1.messages.clone();
        let message = message.clone();
        let username = username.clone();
        Box::pin(async move {
            if message.chars().count() > 100 {
                bot.chat(format!(
                    "/msg {username} {}",
                    message.chars().take(100).collect::<String>()
                ));
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let past_messages =
                    message_state
                        .lock()
                        .await
                        .iter()
                        .take(10)
                        .fold(String::new(), |mut acc, s| {
                            _ = writeln!(&mut acc, "{s}");
                            acc
                        });
                return format!(
                    "# had to truncate message as it was 100+ characters, here is chat history: \n{past_messages}"
                );
            }
            bot.chat(format!("/msg {username} {message}",));
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let past_messages =
                message_state
                    .lock()
                    .await
                    .iter()
                    .take(10)
                    .fold(String::new(), |mut acc, s| {
                        _ = writeln!(&mut acc, "{s}");
                        acc
                    });
            format!(
                "# Successfully sent message, had to truncate, here is chat histor: \n {past_messages}"
            )
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

pub struct StopAgentLoop(Arc<Mutex<bool>>);

impl StopAgentLoop {
    pub fn new(should_stop: Arc<Mutex<bool>>) -> Self {
        Self(should_stop)
    }
}

impl ToolCallFn for StopAgentLoop {
    fn get_name(&self) -> &'static str {
        "stop_agent_loop"
    }

    fn get_description(&self) -> &'static str {
        "stop running the agent loop meaning that you will not continue doing anything after calling this"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let should_stop = self.0.clone();
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
                        _ = writeln!(&mut acc, "[index {}]: {}", idx, slot.kind());
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
        let bot = self.0.clone();
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            if message.chars().count() > 100 {
                let message = message.chars().take(100).collect::<String>();
                bot.chat(format!("/pc {message}"));
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let chat_history =
                    past_messages
                        .lock()
                        .await
                        .iter()
                        .take(10)
                        .fold(String::new(), |mut acc, s| {
                            _ = writeln!(&mut acc, "{s}");
                            acc
                        });
                return format!(
                    "# Message truncated to 100 chars, sent to party. past messages: {chat_history}"
                );
            }
            bot.chat(format!("/pc {message}"));
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let chat_history =
                past_messages
                    .lock()
                    .await
                    .iter()
                    .take(10)
                    .fold(String::new(), |mut acc, s| {
                        _ = writeln!(&mut acc, "{s}");
                        acc
                    });
            format!("# Successfully sent to party chat. past messages:\n{chat_history}")
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
        std::time::Duration::from_millis(200)
    }
}

pub struct LeaveParty<'a>(&'a Client);

impl<'a> LeaveParty<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for LeaveParty<'_> {
    fn get_name(&self) -> &'static str {
        "leave_party"
    }

    fn get_description(&self) -> &'static str {
        "leave your current party"
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        self.0.chat("/p leave");
        "sent leave party command".into_pin_box_string()
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(200)
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
        let bot = self.0.clone();
        let message_queue = self.1.messages.clone();
        Box::pin(async move {
            bot.chat("/p info");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let past_messages = message_queue.lock().await.iter().take(10).fold(
                String::new(),
                |mut acc, message| {
                    _ = writeln!(&mut acc, "{message}");
                    acc
                },
            );

            format!("# requested party info, check chat for details: \n\n{past_messages}")
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        Vec::new()
    }

    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(500)
    }
}

pub struct AddFriend<'a>(&'a Client);

impl<'a> AddFriend<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for AddFriend<'_> {
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
        format!("sent friend request to {username}").into_pin_box_string()
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

pub struct RemoveFriend<'a>(&'a Client);

impl<'a> RemoveFriend<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for RemoveFriend<'_> {
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
        format!("removed {username} from friends").into_pin_box_string()
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
