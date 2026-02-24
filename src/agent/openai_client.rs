use std::collections::{HashMap, hash_map::Values};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

use crate::agent::tool::ToolCallFn;

pub struct OpenAIClient {
    pub url: String,
    pub client: reqwest::Client,
    pub model: String,
    pub header_kv: Option<(String, String)>,
}

pub type BoxedTool<'a> = Box<dyn ToolCallFn + Send + Sync + 'a>;
pub struct ToolMap<'a>(HashMap<&'static str, BoxedTool<'a>>);

impl<'a> ToolMap<'a> {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn values(&self) -> Values<'_, &str, BoxedTool<'_>> {
        self.0.values()
    }

    pub fn register_tool<T: ToolCallFn + Send + Sync + 'a>(mut self, tool: T) -> Self {
        self.0.insert(tool.get_name(), Box::new(tool));
        self
    }

    pub fn get(&self, key: &str) -> Option<&BoxedTool<'_>> {
        self.0.get(key)
    }
}

impl OpenAIClient {
    #[must_use]
    pub fn new(
        url: String,
        client: reqwest::Client,
        model: String,
        header_kv: Option<(String, String)>,
    ) -> Self {
        Self {
            url,
            client,
            model,
            header_kv,
        }
    }

    pub async fn get_completion<'a>(
        &'a self,
        messages: &[ChatCompletionMessageParam],
        tools: &ToolMap<'a>,
    ) -> anyhow::Result<ChatCompletionResponse> {
        let req_tools: Vec<serde_json::Value> =
            tools.values().map(|tool| tool.to_json_value()).collect();
        let request = ChatCompletionRequest {
            model: self.model.clone(),
            tools: if req_tools.is_empty() {
                None
            } else {
                Some(req_tools)
            },
            messages: messages.to_vec(),
        };
        let mut reqbuilder = self
            .client
            .post(format!("{}/chat/completions", self.url))
            .json(&request);
        if let Some((key, token)) = &self.header_kv {
            reqbuilder = reqbuilder.bearer_auth(token).header(key, token);
        }
        let res_str = reqbuilder
            .send()
            .await
            .map_err(|e| anyhow!("no send fail: {e:#?}"))?
            .text()
            .await
            .map_err(|e| anyhow!("no text fail: {e:#?}"))?;
        serde_json::from_str(&res_str)
            .map_err(|e| anyhow!("yeah thing serialisation this is very likely the issue: {e:#?}"))
    }

    pub async fn compact_history(&self, messages: &[ChatCompletionMessageParam]) -> Option<String> {
        let system_prompt: String = "You are a conversation summarizer. Given a conversation between a user and an LLM, produce a compact user prompt that gives the LLM enough context to seamlessly continue from where the conversation left off. Preserve key decisions, unresolved questions, current state, and any relevant artifacts or code.".into();
        let mut history = messages
            .iter()
            .filter(|m| m.role != AIChatRole::System)
            .collect::<Vec<&ChatCompletionMessageParam>>();
        history.dedup_by(|a, b| a.content == b.content);
        let history = history.iter().fold(String::new(), |mut acc, m| {
            _ = writeln!(
                &mut acc,
                "role: {:#?}\ncontent: {}\n\n",
                m.role,
                m.content.trim()
            );
            acc
        });
        let mut first_user_message = messages
            .iter()
            .find_map(|m| {
                if m.role.eq(&AIChatRole::User) {
                    Some(m.content.trim())
                } else {
                    None
                }
            })
            .unwrap_or("could not get initial user request, act based on the history");
        let compacted = self
            .get_completion(
                &[
                    ChatCompletionMessageParam {
                        content: system_prompt,
                        role: AIChatRole::System,
                    },
                    ChatCompletionMessageParam {
                        content: history,
                        role: AIChatRole::User,
                    },
                ],
                &ToolMap::new(),
            )
            .await
            .ok()?
            .choices
            .first()?
            .message
            .clone()
            .content?;
        if first_user_message.contains("now based on that continue fulfilling the initial request:")
        {
            first_user_message = first_user_message
                .split_once("now based on that continue fulfilling the initial request:")
                .map_or(
                    "Failed truncating old usermessage, infer purpose from context",
                    |v| v.1.trim(),
                );
        }
        Some(format!(
            "this is the history of your previous actions compacted: \n\n {compacted} \n now based on that continue fulfilling the initial request: \n\n {first_user_message}"
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub tools: Option<Vec<serde_json::Value>>,
    pub messages: Vec<ChatCompletionMessageParam>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionMessageParam {
    pub content: String,
    pub role: AIChatRole,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AIChatRole {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "tool")]
    Tool,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "assistant")]
    Assistant,
    Custom(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<ChatCompletionResponseChoice>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponseChoice {
    pub message: ChatCompletionResponseMessage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponseMessage {
    pub role: AIChatRole,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolInvocation>>,
    pub reasoning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolInvocation {
    #[serde(rename = "type")]
    pub ftype: String,
    pub function: ToolInvocationFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolInvocationFunction {
    pub arguments: String,
    pub name: String,
}
