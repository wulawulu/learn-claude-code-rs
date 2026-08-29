use anthropic_ai_sdk::{
    client::AnthropicClient,
    types::message::{
        CreateMessageParams, Message, MessageClient, MessageContent, RequiredMessageParams, Role,
    },
};
use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{EvalResult, extract_text, get_model};

#[derive(Debug, Clone)]
pub struct LlmJudgeEvaluator {
    pub rubric: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct JudgeResponse {
    passed: bool,
    score: f32,
    reason: String,
}

impl LlmJudgeEvaluator {
    pub async fn evaluate(
        &self,
        client: &AnthropicClient,
        task: &str,
        agent_answer: &str,
    ) -> Result<EvalResult> {
        let rubric = self
            .rubric
            .iter()
            .enumerate()
            .map(|(index, item)| format!("{}. {item}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            r#"Evaluate the agent answer against the rubric.

Task:
{task}

Agent Answer:
{agent_answer}

Rubric:
{rubric}

Return only one JSON object with exactly this shape:
{{"passed": true, "score": 0.9, "reason": "brief explanation"}}

The score must be a number from 0.0 through 1.0."#
        );
        let model = match std::env::var("EVAL_JUDGE_MODEL") {
            Ok(model) => model,
            Err(_) => get_model()?,
        };
        let request = CreateMessageParams::new(RequiredMessageParams {
            model,
            messages: vec![Message::new_text(Role::User, prompt)],
            max_tokens: 1000,
        })
        .with_system("You are a strict evaluation judge. Follow the rubric and output JSON only.");

        let response = client.create_message(Some(&request)).await?;
        let text = extract_text(&MessageContent::Blocks {
            content: response.content,
        });
        let judge = parse_judge_response(&text)?;

        if !judge.score.is_finite() || !(0.0..=1.0).contains(&judge.score) {
            anyhow::bail!(
                "judge score must be between 0.0 and 1.0, got {}",
                judge.score
            );
        }

        Ok(EvalResult {
            passed: judge.passed,
            score: judge.score,
            reason: judge.reason,
        })
    }
}

fn parse_judge_response(text: &str) -> Result<JudgeResponse> {
    let trimmed = text.trim();
    if let Ok(response) = serde_json::from_str(trimmed) {
        return Ok(response);
    }

    let start = trimmed
        .find('{')
        .context("judge response does not contain a JSON object")?;
    let end = trimmed
        .rfind('}')
        .context("judge response does not contain a complete JSON object")?;

    serde_json::from_str(&trimmed[start..=end]).context("failed to parse judge JSON response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let response =
            parse_judge_response(r#"{"passed":true,"score":0.9,"reason":"meets the rubric"}"#)
                .unwrap();

        assert!(response.passed);
        assert_eq!(response.score, 0.9);
    }

    #[test]
    fn parses_json_code_fence() {
        let response = parse_judge_response(
            "```json\n{\"passed\":false,\"score\":0.4,\"reason\":\"missing a fix\"}\n```",
        )
        .unwrap();

        assert!(!response.passed);
        assert_eq!(response.reason, "missing a fix");
    }
}
