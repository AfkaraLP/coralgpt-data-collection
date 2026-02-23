use azalea::Client;
use std::{fmt::Write, pin::Pin};

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

pub struct SendChatMessage<'a>(&'a Client);

impl<'a> SendChatMessage<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self(client)
    }
}

impl ToolCallFn for SendChatMessage<'_> {
    fn get_name(&self) -> &'static str {
        "send_chat_message"
    }

    fn get_description(&self) -> &'static str {
        "send a message to global chat as the bot"
    }

    // async fn invoke(&self, args: &serde_json::Value) -> Pin<Box<>> {
    //     let Some(message_val) = args.get("message") else {
    //         return "please provide a \"message\" field in the tool call".into();
    //     };
    //     let serde_json::Value::String(message) = message_val else {
    //         return "message has to be of type string".into();
    //     };

    //     if message.chars().count() > 100 {
    //         let message = message.chars().take(100).collect::<String>();
    //         self.0.chat(message);
    //         return "message too long, truncating to 100 chars and sending anyway".into();
    //     }
    //     self.0.chat(message);
    //     "successfully sent message in global chat".into()
    // }

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

        if message.chars().count() > 100 {
            let message = message.chars().take(100).collect::<String>();
            self.0.chat(message);
            return "message too long, truncating to 100 chars and sending anyway"
                .into_pin_box_string();
        }
        self.0.chat(message);
        "successfully sent message in global chat".into_pin_box_string()
    }
}

pub struct RespondToUser<'a, 'b>(&'a Client, &'b str);

impl<'a, 'b> RespondToUser<'a, 'b> {
    pub fn new(client: &'a Client, user: &'b str) -> Self {
        Self(client, user)
    }
}

impl ToolCallFn for RespondToUser<'_, '_> {
    fn get_name(&self) -> &'static str {
        "respond_to_user"
    }

    fn get_description(&self) -> &'static str {
        "respond to the user as bot ingame in private messages"
    }

    // async fn invoke(&self, args: &serde_json::Value) -> String {
    //     let Some(message_val) = args.get("message") else {
    //         return "please provide a \"message\" field in the tool call".into();
    //     };
    //     let serde_json::Value::String(message) = message_val else {
    //         return "message has to be of type string".into();
    //     };

    //     if message.chars().count() > 100 {
    //         let message = message.chars().take(100).collect::<String>();
    //         self.0.chat(message);
    //         return "message too long, truncating to 100 chars and sending anyway".into();
    //     }
    //     self.0.chat(message);
    //     "successfully sent message to user ingame".into()
    // }

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

        if message.chars().count() > 100 {
            let message = message.chars().take(100).collect::<String>();
            self.0.chat(format!("/msg {} {message}", self.1));
            return "message too long, truncating to 100 chars and sending anyway"
                .into_pin_box_string();
        }
        self.0.chat(format!("/msg {} {message}", self.1));
        "successfully sent message to user ingame".into_pin_box_string()
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

    // async fn invoke(&self, _args: &serde_json::Value) -> String {
    //     self.0
    //         .tab_list()
    //         .values()
    //         .fold(String::new(), |mut acc, val| {
    //             if let Some(name) = &val.display_name {
    //                 _ = writeln!(&mut acc, "{}", name.to_string());
    //             }
    //             acc
    //         })
    // }

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

    // async fn invoke(&self, args: &serde_json::Value) -> String {
    //     let num_messages = match args.get("count") {
    //         Some(count) => match count {
    //             serde_json::Value::Number(number) => number.as_u64().unwrap_or(20),
    //             _ => 20,
    //         },
    //         None => 20,
    //     };
    //     self.0
    //         .messages
    //         .lock()
    //         .await
    //         .iter()
    //         .rev()
    //         .take(num_messages.try_into().unwrap_or(20))
    //         .fold(String::new(), |mut acc, v| {
    //             writeln!(&mut acc, "{v}");
    //             acc
    //         })
    // }

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
                .rev()
                .take(num_messages.try_into().unwrap_or(20))
                .fold(String::new(), |mut acc, v| {
                    _ = writeln!(&mut acc, "{v}");
                    acc
                })
        })
    }
}
