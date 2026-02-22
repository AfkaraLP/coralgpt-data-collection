use std::collections::HashMap;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::agent::tool::ToolCallFn;

pub struct OpenAIClient {
    pub url: String,
    pub client: reqwest::Client,
    pub model: String,
    pub header_kv: Option<(String, String)>,
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
        messages: &Vec<ChatCompletionMessageParam>,
        tools: &HashMap<String, Box<dyn ToolCallFn + Send + Sync + 'a>>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
