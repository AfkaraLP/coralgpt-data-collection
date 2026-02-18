use std::collections::HashMap;

pub trait ToolCallFn {
    #[must_use]
    fn get_name(&self) -> String;
    #[must_use]
    fn get_description(&self) -> String;

    fn invoke(&self, args: serde_json::Value) -> String;

    #[must_use]
    fn to_json_value(&self) -> serde_json::Value {
        let name = self.get_name();
        let description = self.get_description();
        let args = self.get_args();
        let required = args
            .iter()
            .filter_map(|arg| {
                if arg.required {
                    Some(arg.name.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<String>>();
        let props: HashMap<String, serde_json::Value> =
            args.iter().map(|v| v.serialize()).collect();
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
}

pub struct ToolCallArgDescriptor {
    pub required: bool,
    pub name: String,
    pub argtype: String,
    pub description: String,
}

impl ToolCallArgDescriptor {
    #[must_use]
    pub fn new(required: bool, name: String, argtype: String, description: String) -> Self {
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
            self.name.clone(),
            serde_json::json!({
                "type": self.argtype,
                "description": self.description
            }),
        )
    }
}
