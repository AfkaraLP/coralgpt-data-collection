use std::{collections::HashMap, pin::Pin};

use serde::{Deserialize, Serialize};

pub trait ToolCallFn {
    #[must_use]
    fn get_name(&self) -> &'static str;
    #[must_use]
    fn get_description(&self) -> &'static str;

    fn invoke(&self, args: &serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send>>;

    #[must_use]
    fn to_json_value(&self) -> serde_json::Value {
        let name = self.get_name();
        let description = self.get_description();
        let args = self.get_args();
        let required = args
            .iter()
            .filter_map(|arg| if arg.required { Some(arg.name) } else { None })
            .collect::<Vec<&str>>();
        let props: HashMap<String, serde_json::Value> =
            args.iter().map(ToolCallArgDescriptor::serialize).collect();
        let json = serde_json::json!(
            {
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": {
                        "type": "object",
                        "properties": props,
                        "required": required
                    }
                }
            }
        );
        json
    }

    #[must_use]
    fn get_args(&self) -> Vec<ToolCallArgDescriptor>;

    #[must_use]
    fn get_timeout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_millis(3000)
    }
}

pub struct ToolCallArgDescriptor {
    pub required: bool,
    pub name: &'static str,
    pub argtype: ToolCallArgType,
    pub description: &'static str,
}

#[derive(Serialize, Deserialize)]
pub enum ToolCallArgType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
}

impl ToolCallArgDescriptor {
    pub fn new(
        required: bool,
        name: &'static str,
        argtype: ToolCallArgType,
        description: &'static str,
    ) -> Self {
        Self {
            required,
            name,
            argtype,
            description,
        }
    }

    #[must_use]
    fn serialize(&self) -> (String, serde_json::Value) {
        (
            self.name.to_string(),
            serde_json::json!({
                "type": self.argtype,
                "description": self.description
            }),
        )
    }
}
