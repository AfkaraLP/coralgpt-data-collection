#![allow(clippy::cast_precision_loss)]

use azalea::{
    BlockPos, Client, Vec3,
    pathfinder::{PathfinderOpts, goals::BlockPosGoal},
    prelude::PathfinderClientExt,
};
use openai_client::{ChatCompletionMessageParam, ToolCallArgDescriptor, ToolCallFn, ToolMap};
use std::{fmt::Write, pin::Pin, time::Duration};
use tokio::{
    sync::Mutex,
    time::{Instant, sleep},
};

const ONE_POINT_EIGHT_MIN_Y: i32 = 0;

pub trait IntoPinBoxString<'a>: Sized + Send + ToString + 'a {
    fn into_pin_box_string(self) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move { self.to_string() })
    }
}

impl<'a, T: Sized + Send + ToString + 'a> IntoPinBoxString<'a> for T {}

use crate::{ENV_VARS, State, buffer::ChatFifoOps, is_spam_message};

const SUBAGENT_PROMPT: &str = include_str!("./SUBAGENT.md");
pub struct SpawnSubAgent<'a> {
    subagent_tools: &'a ToolMap<'a>,
}

impl<'a> SpawnSubAgent<'a> {
    pub fn new(subagent_tools: &'a ToolMap<'a>) -> Self {
        Self { subagent_tools }
    }
}

impl ToolCallFn for SpawnSubAgent<'_> {
    fn get_name(&self) -> &'static str {
        "spawn_sub_agent"
    }

    fn get_description(&self) -> &'static str {
        "Spawn a specialized AI subagent that can autonomously control the bot and perform complex multi-step tasks. The subagent has access to all the same tools as the main agent and can reason through problems that require multiple actions.\n\nUse this when:\n- You need to perform a complex task that requires planning (e.g., \"go find a specific resource and bring it back\")\n- You need to explore the world and report back findings\n- You need to complete a multi-step objective that would require many individual tool calls\n- You want the bot to make decisions based on changing conditions\n\nExample: If someone asks \"Can you go to the spawn and check if the shop is open?\", spawn a subagent with that task."
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
        let agent_loop = async move {
            let Some(client) = &ENV_VARS.openai_client else {
                return "for some reason agent is not available right now".into();
            };
            let subagent_response = client
                .run_agent(
                    vec![
                        ChatCompletionMessageParam::new_system(SUBAGENT_PROMPT),
                        ChatCompletionMessageParam::new_user(&task),
                    ],
                    self.subagent_tools,
                )
                .await
                .map_or_else(
                    |_| String::from("Subagent Run Failed"),
                    |(res, _)| res.content.unwrap_or_default(),
                );
            eprintln!("subagent summary: {subagent_response}");
            subagent_response
        };
        Box::pin(agent_loop)
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::string(
            "task",
            "A detailed description of the task you want the subagent to complete. Be specific about what success looks like. The subagent will keep running until it completes the task or determines it cannot be done.\n\nExample: \"Go to coordinates [100, 64, 200], search for a diamond ore block within 10 blocks, and tell me what you found\"",
        )]
    }

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Send a text message to the global/public chat visible to all players in the current lobby. This is the main way to communicate with anyone in the game.\n\nUse this when:\n- Responding to public questions\n- Starting a conversation in global chat\n- Making public announcements\n- Talking to strangers\n\nNote: Messages are limited to 100 characters. The tool returns the past 8 chat messages to help you see the context of your message."
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::string(
            "message",
            "The text message to send to global chat. Must be 100 characters or less. This appears publicly to all players in the lobby.\n\nExample: \"Hey everyone! Looking for a duo game\"",
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
            sleep(Duration::from_secs(1)).await;
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

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_secs(15)
    }
}

pub struct RespondToUser<'a, 'b, 'c>(&'a Client, &'b str, &'c State);

impl<'a, 'b, 'c> RespondToUser<'a, 'b, 'c> {
    pub fn new(client: &'a Client, username: &'b str, state: &'c State) -> Self {
        Self(client, username, state)
    }
}

impl ToolCallFn for RespondToUser<'_, '_, '_> {
    fn get_name(&self) -> &'static str {
        "respond_to_user"
    }

    fn get_description(&self) -> &'static str {
        "Send a private message (DM) directly to a specific player. This is a private, one-on-one conversation that only the recipient can see.\n\nUse this when:\n- Someone messages you privately and you want to reply\n- You want to have a private conversation without others seeing\n- Responding to /msg commands from other players\n- Sending sensitive information you don't want others to see\n\nNote: Limited to 100 characters. Returns the past 5 messages to see your message in context."
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::string(
            "message",
            "The private message to send to the specific player. This goes directly to them via /msg. Max 100 characters.\n\nExample: \"Sure, I'll join your party!\"",
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
        let username = self.1;
        let past_messages = self.2.messages.clone();
        self.0.chat(format!("/msg {username} {message}"));
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        "Retrieve the list of all players currently in the same lobby/server as you. The tab list typically shows player names along with their rank, guild tag, and other metadata.\n\nUse this when:\n- Checking who is online in your lobby\n- Looking for specific players to invite to party/games\n- Seeing if friends are online\n- Checking for potential teammates or opponents\n\nNote: This only shows players in your current server/lobby, not the entire network."
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
        "Retrieve recent messages from the chat history. This allows you to see what has been said recently, including public messages, private messages sent to you, system announcements, and join/leave messages.\n\nUse this when:\n- Reviewing recent conversations to understand context\n- Checking if someone responded to you\n- Seeing what happened while you were away\n- Finding specific information someone said earlier\n\nTip: Use the 'verbose' parameter to include spam messages like server announcements and join/leave messages. Try to request only as many messages as you need to avoid filling up context too quickly."
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::number(
                "count",
                "The number of past messages to retrieve. This determines how far back in chat history to look. Higher values give more context but use more tokens. Recommended: 5-20 for most situations.\n\nExample: 10 (get the 10 most recent messages)",
            ),
            ToolCallArgDescriptor::bool(
                "verbose",
                "When set to true, includes all messages including server announcements, join/leave messages, and other spam. When false (default), filters out spam messages to show only meaningful player messages.\n\nExample: false to see only player chat, true to see everything",
            ),
        ]
    }

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Pause the agent's execution for a specified duration. This is useful when you need to wait for something to happen in the game, such as waiting for a party invite, waiting for a game to start, or letting chat messages appear.\n\nUse this when:\n- Waiting for a party invite to be sent/accepted\n- Waiting for a game to load\n- Pausing between actions to let the world update\n- Giving time for chat messages to appear\n- Waiting for cooldowns\n\nNote: The agent will pause but will still return any new chat messages that arrived during the sleep."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        match args.get("time") {
            Some(serde_json::Value::Number(num)) => {
                if let Some(num) = num.as_u64() {
                    let message_state = self.0.messages.clone();
                    Box::pin(async move {
                        sleep(Duration::from_secs(num)).await;
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
        vec![ToolCallArgDescriptor::number(
            "time",
            "The duration to sleep in seconds. This is an unsigned integer representing how long to wait.\n\nExample: 5 (wait for 5 seconds), 10 (wait for 10 seconds)\n\nTip: 1-3 seconds is usually enough for chat to update, 5-10 seconds for waiting for party/game actions.",
        )]
    }

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Join an existing party created by another player. You can join parties from players who have invited you or from players who have set their party to public/open.\n\nUse this when:\n- Someone invites you to their party and you want to accept\n- A player has a public party and you want to join\n- You want to play with a specific player\n\nNote: Returns confirmation based on chat feedback - checks for success message or error messages like \"No party request from this player\"."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "did not provide correct arguments for party join".into_pin_box_string();
        };
        self.0.chat(format!("/p join {username}"));
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player whose party you want to join. This player must have either invited you or have an open/public party.\n\nExample: \"Player123\" (join Player123's party)\n\nTip: You can check who is in the lobby with read_tab_list to find players with open parties.",
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
        "Invite a player to join your party. If you don't already have a party, this will create a new party and add the invited player. Parties allow you to play together and coordinate.\n\nUse this when:\n- You want to play with a friend and need to invite them\n- Creating a party to start a team game (duos, trios)\n- Adding players to your existing party\n\nNote: Returns success if invite sent, or error messages like \"player offline\", \"already invited\", or \"player in another party\"."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "did not provide correct arguments for party invite".into_pin_box_string();
        };
        self.0.chat(format!("/p invite {username}"));
        let past_messages = self.1.messages.clone();
        let username = username.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player you want to invite to your party. The player must be online for the invite to work.\n\nExample: \"FriendPlayer\" (invite FriendPlayer to join your party)\n\nTip: Check if the player is online using read_tab_list first.",
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
        "Send a private message (DM) to any online player. This is similar to respond_to_user but allows you to initiate a conversation with any player rather than just responding to one who already messaged you.\n\nUse this when:\n- You want to start a private conversation with a specific player\n- Sending information to a player who isn't currently messaging you\n- Coordinating with teammates privately\n- Asking a player something without others seeing\n\nNote: Player must be online. Max 100 characters. Returns feedback based on whether the message was delivered."
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
            sleep(Duration::from_secs(1)).await;

            let messages: Vec<String> = message_state
                .lock()
                .await
                .iter()
                .map(ToString::to_string)
                .filter(|v| !is_spam_message(v))
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
            ToolCallArgDescriptor::string(
                "username",
                "The username of the player to send the direct message to. The player must be online for the message to go through.\n\nExample: \"PlayerName\"",
            ),
            ToolCallArgDescriptor::string(
                "message",
                "The private message content to send. Max 100 characters. This goes directly to the player's inbox.\n\nExample: \"Hey, want to play duo?\"",
            ),
        ]
    }
}

pub struct StopAgentLoop<'a>(std::sync::Arc<Mutex<bool>>, String, &'a Client);

impl<'a> StopAgentLoop<'a> {
    pub fn new(mutex: std::sync::Arc<Mutex<bool>>, username: String, client: &'a Client) -> Self {
        Self(mutex, username, client)
    }
}

impl ToolCallFn for StopAgentLoop<'_> {
    fn get_name(&self) -> &'static str {
        "stop_agent_loop"
    }

    fn get_description(&self) -> &'static str {
        "Stop the agent loop and terminate all further execution. This is the proper way to end the agent's session when you're done or want to exit.\n\nUse this when:\n- Task is complete and you want to cleanly exit\n- Something went wrong and you want to stop\n- User tells you to stop/leave\n- You want to log off or disconnect\n\nNote: This will inform the user that you're stopping and then terminate the loop. Always use this when done rather than just stopping abruptly."
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

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "View your current inventory contents. This shows all items you are carrying, including items in your inventory and hotbar. Slots are numbered starting from 0 at the top-left, going row by row.\n\nUse this when:\n- Checking what items you have\n- Finding a specific item to use\n- Seeing your resources/materials\n- Before selecting a hotbar slot\n\nNote: Slot indices 0-8 are the hotbar (slots 36-44 in Minecraft's internal system). The inventory displays items from top-left to bottom-right."
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

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Select which slot from your hotbar to hold as your current main hand item. This changes what item you're actively using when you interact with the world.\n\nUse this when:\n- You need to switch to a specific tool (pickaxe, sword, etc.)\n- Want to use a particular item\n- Preparing to mine/place blocks\n- Switching to combat items\n\nNote: Slots are indexed 0-8. Slot 0 is the leftmost hotbar slot, slot 8 is the rightmost. The index wraps around (selecting index 9 goes to slot 0)."
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
        vec![ToolCallArgDescriptor::number(
            "index",
            "The hotbar slot index to select (0-8). 0 is the first slot on the left, 8 is the last on the right. The value wraps around (9 becomes 0, 10 becomes 1, etc.).\n\nExample: 0 (select the leftmost slot), 4 (select the middle slot)\n\nTip: Use get_inventory first to see what items are in each slot.",
        )]
    }

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Send a message to your party's chat. This message is only visible to other members of your party, not to the public chat.\n\nUse this when:\n- Coordinating with party members\n- Planning strategy privately\n- Telling teammates what you're doing\n- Asking party members to do something\n\nNote: Only works if you're in a party. Returns an error if you're not in a party. Max 100 characters."
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
            sleep(Duration::from_secs(1)).await;
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
        vec![ToolCallArgDescriptor::string(
            "message",
            "The message to send to your party members. This is private and only party members can see it. Max 100 characters.\n\nExample: \"Let's go to mid!\"",
        )]
    }

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_secs(1)
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
        "Leave your current party. This removes you from the party you're in, whether you're the leader or just a member.\n\nUse this when:\n- You want to leave a party and play solo\n- Party leader is afk/hostile and you want to leave\n- You're done playing with the current group\n- Starting a new party with different players\n\nNote: If you're the leader, the party will be disbanded for all members. Returns confirmation based on chat message."
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        self.0.chat("/p leave");
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        "Get detailed information about your current party, including the party leader, all members, whether the party is open/public, and if it's muted.\n\nUse this when:\n- Checking who is in your party\n- Seeing if party is open so others can join\n- Confirming a player joined your party\n- Checking party settings\n\nNote: Returns \"You are not in a party\" if you're not currently in a party. Returns a formatted summary of party details."
    }

    fn invoke(&self, _args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        self.0.chat("/p info");
        let message_queue = self.1.messages.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        "Send a friend request to a player. Once accepted, you can see when they're online and potentially play together.\n\nUse this when:\n- You meet a player you want to keep track of\n- Someone asks to be friends\n- You want to remember a good teammate\n\nNote: Returns success/failure based on server response. The friend request stays pending until the other player accepts or declines."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username to add as friend".into_pin_box_string();
        };
        self.0.chat(format!("/f add {username}"));
        let past_messages = self.1.messages.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player to send a friend request to. The player doesn't need to be online to send the request.\n\nExample: \"CoolPlayer123\"",
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
        "Remove a player from your friends list. This is the opposite of add_friend - it unfriends someone.\n\nUse this when:\n- You no longer want to be friends with someone\n- Freeing up friend slots (if limited)\n- Removing someone who was added by mistake\n\nNote: Returns confirmation or error if you're not friends with that player."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username to remove from friends".into_pin_box_string();
        };
        self.0.chat(format!("/f remove {username}"));
        let past_messages = self.1.messages.clone();
        let username = username.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player to remove from your friends list.\n\nExample: \"ExFriend123\"",
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
        "Retrieve detailed Bedwars player statistics for any player. This shows their performance metrics including level, wins, kills, K/D ratio, beds broken, and more.\n\nUse this when:\n- Checking your own stats\n- Viewing another player's stats to see their skill level\n- Comparing players\n- Seeing someone's winstreak or division\n\nNote: Returns a formatted summary of the player's stats. Filters out spam messages to find the stats data."
    }

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let Some(serde_json::Value::String(username)) = args.get("username") else {
            return "please provide a username string for whose stats to get".into_pin_box_string();
        };
        self.0.chat(format!("/stats {username}"));
        let past_messages = self.1.messages.clone();
        let username = username.clone();
        Box::pin(async move {
            sleep(Duration::from_secs(1)).await;
            let past_messages = past_messages.lock().await;
            let past_messages = past_messages
                .iter()
                .filter(|v| !is_spam_message(v.to_string()))
                .take(25);
            let stats = PlayerStats::parse_from_chat(past_messages.map(ToString::to_string));
            let stats_text = stats.player_stats_to_llm_text();
            println!("got stats: {stats_text:#?}");
            format!("{username}'s stats: {stats_text}")
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player whose stats you want to view. Can be your own username or another player's.\n\nExample: \"YourUsername\" (check your own stats), \"ProPlayer123\" (check someone else's)",
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
        "Scan and list blocks in the area around your current position. This allows you to see what blocks are nearby - useful for finding resources, navigating, or understanding your environment.\n\nUse this when:\n- Looking for specific blocks (ores, wood, etc.)\n- Navigating and understanding your surroundings\n- Finding resources to collect\n- Checking what's around you before moving\n\nNote: Default radius is 5 blocks in X/Y and 3 in Z. Use block_type to filter for specific blocks. Air blocks are excluded from results."
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
            ToolCallArgDescriptor::number(
                "z_radius",
                "How far in the Z axis (north/south) to scan for blocks. Default is 3. Positive and negative values scan in both directions from your position.\n\nExample: 5 (scan 5 blocks in each Z direction)",
            ),
            ToolCallArgDescriptor::number(
                "x_radius",
                "How far in the X axis (east/west) to scan for blocks. Default is 5. Scans in both directions from your position.\n\nExample: 10 (scan 10 blocks in each X direction)",
            ),
            ToolCallArgDescriptor::number(
                "y_radius",
                "How far in the Y axis (up/down) to scan for blocks. Default is 5. Scans both above and below your position.\n\nExample: 3 (scan 3 blocks up and down)",
            ),
            ToolCallArgDescriptor::string(
                "block_type",
                "Filter to only show a specific type of block. Use Minecraft block names in lowercase with underscores. If not provided, shows all non-air blocks in range.\n\nExample values: \"oak_log\", \"diamond_ore\", \"glass\", \"stone\", \"cobblestone\"\n\nTip: Use this to quickly find specific resources like 'diamond_ore' or 'gold_ore'",
            ),
        ]
    }

    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Navigate the bot to a specific coordinate position using the built-in pathfinder. The pathfinder will find a walkable route to the target position.\n\nUse this when:\n- Moving to a specific location\n- Going to coordinates you got from get_nearby_blocks\n- Navigating to a destination\n- Finding your way around\n\nNote: The pathfinder cannot use stairs. Make sure the destination is reachable by walking. Returns success when within 1.5 blocks of target, or timeout/distance error if can't reach."
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
                                Duration::from_secs(1),
                            ),
                        ),
                    )
                    .await;
            });

            if tokio::time::timeout(Duration::from_millis(1500), async {
                while bot.is_calculating_path() {
                    sleep(Duration::from_secs(1)).await;
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

                sleep(Duration::from_secs(1)).await;
            };
            bot.stop_pathfinding();
            goto_thread.abort();
            response
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::number(
                "x",
                "The X coordinate (east-west axis) of the target position to pathfind to. This is a signed integer.\n\nExample: 100 (move to x=100)",
            ),
            ToolCallArgDescriptor::number(
                "y",
                "The Y coordinate (vertical axis) of the target position. This is the height/elevation. Must be a valid block position.\n\nExample: 64 (move to y=64, typical ground level)",
            ),
            ToolCallArgDescriptor::number(
                "z",
                "The Z coordinate (north-south axis) of the target position to pathfind to. This is a signed integer.\n\nExample: -50 (move to z=-50)",
            ),
            ToolCallArgDescriptor::number(
                "time_to_reach_goal",
                "Maximum time in seconds to spend trying to reach the destination before giving up. Default is 10 seconds. Increase for longer distances.\n\nExample: 30 (give up after 30 seconds if not reached)",
            ),
        ]
    }
    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Get your current 3D coordinates (X, Y, Z) in the game world. This tells you exactly where the bot is positioned.\n\nUse this when:\n- Finding out where you are\n- Getting coordinates to use with path_find\n- Checking if you reached a destination\n- Understanding your location relative to something\n\nNote: Y coordinate is height (0 is bedrock, 64 is sea level in the overworld)."
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
        "Scan the area around you and return a deduplicated list of unique block types present. Unlike get_nearby_blocks which shows exact positions, this gives you just the types of blocks you can find nearby.\n\nUse this when:\n- Quickly checking what resources are available nearby\n- Getting a summary of your surroundings\n- Looking for specific block types without needing positions\n\nNote: Default radius is 5 blocks in X/Y, 3 in Z. Shows only unique block types (no duplicates in the list)."
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
            ToolCallArgDescriptor::number(
                "z_radius",
                "How far in the Z axis (north/south) to scan. Default is 3. Scans in both directions.\n\nExample: 5",
            ),
            ToolCallArgDescriptor::number(
                "x_radius",
                "How far in the X axis (east/west) to scan. Default is 5. Scans in both directions.\n\nExample: 10",
            ),
            ToolCallArgDescriptor::number(
                "y_radius",
                "How far in the Y axis (up/down) to scan. Default is 5. Scans both above and below.\n\nExample: 3",
            ),
        ]
    }
    fn get_timeout_wait(&self) -> Duration {
        Duration::from_millis(0)
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
        "Decline a pending friend request you received from another player. This rejects their request to add you as a friend.\n\nUse this when:\n- Someone sent you a friend request you don't want to accept\n- Clearing out unwanted friend requests\n\nNote: You must have received a request from this player to decline it."
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player whose friend request you want to decline.\n\nExample: \"UnknownPlayer\"",
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
        "Accept a pending friend request you received from another player. Once accepted, they will be added to your friends list.\n\nUse this when:\n- Someone sent you a friend request and you want to accept it\n- Adding a player as a friend\n\nNote: You must have received a request from this player to accept it."
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player whose friend request you want to accept.\n\nExample: \"NewFriend123\"",
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
        "Execute any Minecraft server command directly. This gives you flexibility to run commands that don't have dedicated tools.\n\nUse this when:\n- Running a command not covered by other tools\n- Accessing server-specific features\n- Using administrative commands\n- Running custom server commands\n\nNote: Returns the chat output/response from the server. Use this when you know the exact command syntax for the server."
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
            sleep(Duration::from_secs(1)).await;
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
                "Unknown Command".into()
            } else {
                format!("Ran command, past chat output:\n\n{past_messages}")
            }
        })
    }

    fn get_args(&self) -> Vec<ToolCallArgDescriptor> {
        vec![
            ToolCallArgDescriptor::string(
                "command",
                "The main command category to run (without the slash). This is the command name.\n\nExample: \"warp\", \"spawn\", \"shop\", \"help\"",
            ),
            ToolCallArgDescriptor::string(
                "args",
                "Additional arguments to pass to the command. This is everything after the command name.\n\nExample: \"nether\" (to run /warp nether), \"playerName\" (to run /stats playerName)\n\nLeave empty if the command takes no arguments.",
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
        "Run commonly used quick commands for game navigation and basic status checks. This is a simplified interface for frequent actions like going to spawn, joining games, checking stats, etc.\n\nUse this when:\n- Quickly going to spawn\n- Joining duo/trio games\n- Checking your own stats\n- Checking your ping\n\nThis is a convenient shortcut - these are the same commands you'd run via run_command but easier to access."
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
            sleep(Duration::from_secs(1)).await;
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
        vec![ToolCallArgDescriptor::string(
            "command",
            "The quick command to run. Choose from:\n\n- \"spawn\": Teleport to the lobby spawn location\n- \"playtwo\": Join a duo (2-player) game\n- \"playthree\": Join a trio (3-player) game  \n- \"stats\": View your own Bedwars stats\n- \"ping\": Check your current network latency\n\nExample: \"spawn\" (go to lobby), \"playtwo\" (join duo game)",
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
        "Join a private game hosted by another player. This allows you to join games that players have made private and shared the link/code for.\n\nUse this when:\n- Someone shares their private game with you\n- Joining a friend's custom game\n- Entering a specific private lobby\n\nNote: The player must have an active private game and you need their username. Returns error if no private game found for that player."
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
        vec![ToolCallArgDescriptor::string(
            "username",
            "The username of the player whose private game you want to join. They must currently be in a private game for this to work.\n\nExample: \"GameHost123\"",
        )]
    }
}
