//! Classifier-slot LLM caller used by the LLM narration path.
//!
//! The hook hands the tool name + redacted args to a small classifier
//! model (~50-max-token cap, `tool_choice: none`) and returns the
//! one-sentence narration. Timeouts and LLM errors surface as a string
//! reason that the hook converts into a `Misc{kind:"narration_skipped"}`
//! event.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::llm::{ChatCompletionRequest, LlmClient, Message, Role, ToolChoice, ToolChoiceMode};

const MAX_TOKENS: u32 = 50;

#[derive(Clone)]
pub struct ClassifierSlot {
    pub llm: Arc<LlmClient>,
    pub model: String,
    pub system_prompt: Arc<String>,
}

impl ClassifierSlot {
    pub async fn classify(
        &self,
        tool: &str,
        args: &Value,
        timeout: Duration,
    ) -> Result<String, String> {
        let user_content = format!(
            "Tool: {tool}\nArgs: {}\nWrite ONE 8-15 word narration sentence for the user.",
            args
        );
        let req = ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some(self.system_prompt.as_str().to_string()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some(user_content),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: None,
            tool_choice: Some(ToolChoice::String(ToolChoiceMode::None)),
            temperature: None,
            max_tokens: Some(MAX_TOKENS),
            top_p: None,
        };

        let resp = tokio::time::timeout(timeout, self.llm.chat_completion(req))
            .await
            .map_err(|_| "timeout".to_string())?
            .map_err(|e| format!("llm: {e}"))?;

        let text = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| "no_content".to_string())?;
        Ok(strip_quotes_and_period(text.trim()).to_string())
    }
}

fn strip_quotes_and_period(s: &str) -> &str {
    let trimmed = s.trim_matches(|c: char| c == '"' || c == '\'').trim();
    trimmed.trim_end_matches('.').trim()
}
