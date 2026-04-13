use anthropic_ai_sdk::types::message::{ContentBlock, Message, MessageContent, Role};
use anyhow::Context as _;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anthropic_ai_sdk::types::message::{CreateMessageParams, MessageClient, RequiredMessageParams};

use crate::{LoopState, MODEL};

const KEEP_RECENT_TOOL_RESULTS: usize = 3;
const PERSIST_THRESHOLD: usize = 30000;
const PREVIEW_CHARS: usize = 2000;
const TRANSCRIPT_DIR: &str = ".transcripts";
const OUTPUT_DIR: &str = ".task_outputs/tool-results";
const COMPACTED_TOOL_RESULT: &str =
    "[Earlier tool result compacted. Re-run the tool if you need full detail.]";

#[derive(Debug, Default)]
pub struct CompactState {
    pub has_compacted: bool,
    pub last_summary: Option<String>,
    pub recent_files: Vec<String>,
}

pub fn micro_compact(messages: &mut [Message]) {
    let tool_result_positions = collect_tool_result_positions(messages);
    if tool_result_positions.len() <= KEEP_RECENT_TOOL_RESULTS {
        return;
    }

    let compact_until = tool_result_positions.len() - KEEP_RECENT_TOOL_RESULTS;

    for (message_idx, block_idx) in tool_result_positions.into_iter().take(compact_until) {
        let Some(message) = messages.get_mut(message_idx) else {
            continue;
        };

        let MessageContent::Blocks { content } = &mut message.content else {
            continue;
        };

        let Some(ContentBlock::ToolResult {
            content: tool_content,
            ..
        }) = content.get_mut(block_idx)
        else {
            continue;
        };

        if tool_content.chars().count() > 120 {
            *tool_content = COMPACTED_TOOL_RESULT.to_string();
        }
    }
}

pub fn estimate_context_size(messages: &[Message]) -> usize {
    serde_json::to_string(messages)
        .map(|serialized| serialized.chars().count())
        .unwrap_or_else(|_| {
            messages
                .iter()
                .map(|message| match &message.content {
                    MessageContent::Text { content } => content.chars().count(),
                    MessageContent::Blocks { content } => content
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Text { text } => text.chars().count(),
                            ContentBlock::ToolUse { name, input, .. } => {
                                name.chars().count() + input.to_string().chars().count()
                            }
                            ContentBlock::ToolResult { content, .. } => content.chars().count(),
                            _ => 0,
                        })
                        .sum(),
                })
                .sum::<usize>()
        })
}

pub fn write_transcript(messages: &[Message]) -> anyhow::Result<PathBuf> {
    let transcript_dir = std::env::current_dir()?.join(TRANSCRIPT_DIR);
    fs::create_dir_all(&transcript_dir)
        .with_context(|| format!("failed to create {}", transcript_dir.display()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs();
    let transcript_path = transcript_dir.join(format!("transcript_{timestamp}.jsonl"));

    let file = File::create(&transcript_path)
        .with_context(|| format!("failed to create {}", transcript_path.display()))?;
    let mut writer = BufWriter::new(file);

    for message in messages {
        serde_json::to_writer(&mut writer, message).with_context(|| {
            format!(
                "failed to serialize message to {}",
                transcript_path.display()
            )
        })?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;

    Ok(transcript_path)
}

pub fn persist_large_output(tool_use_id: &str, output: &str) -> anyhow::Result<String> {
    if output.chars().count() <= PERSIST_THRESHOLD {
        return Ok(output.to_string());
    }

    let output_dir = std::env::current_dir()?.join(OUTPUT_DIR);
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let output_path = output_dir.join(format!("{tool_use_id}.txt"));

    fs::write(&output_path, output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    let output_path = output_path.display();

    let preview = output.chars().take(PREVIEW_CHARS).collect::<String>();
    Ok(format!(
        "<persisted-output>\nFull output saved to: {output_path}\nPreview:\n{preview}\n</persisted-output>"
    ))
}

impl LoopState {
    pub async fn compact_history(&mut self, focus: Option<&str>) -> anyhow::Result<()> {
        let transcript_path =
            write_transcript(&self.context).context("failed to write transcript")?;
        println!("[transcript saved: {}]", transcript_path.display());

        let mut summary = self
            .summarize_history()
            .await
            .context("failed to summarize history")?;
        if let Some(focus) = focus {
            summary = format!("{summary}\n\nFocus to preserve next:{focus}");
        }
        if !self.compact_state.recent_files.is_empty() {
            let recent_lines = self
                .compact_state
                .recent_files
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n");
            summary = format!("{summary}\n\nRecent files to reopen if needed:\n{recent_lines}");
        }

        self.compact_state.has_compacted = true;
        self.compact_state.last_summary = Some(summary.clone());

        self.context = vec![Message::new_text(
            Role::User,
            format!(
                "This conversation was compacted so the agent can continue working.\n\n{summary}"
            ),
        )];
        Ok(())
    }

    pub async fn summarize_history(&self) -> anyhow::Result<String> {
        let conversation_text = serde_json::to_string(&self.context)
            .context("failed to serialize conversation for summarization")?;
        let truncated = conversation_text.chars().take(80000).collect::<String>();

        let prompt = format!(
            "Summarize this coding-agent conversation so work can continue.\n\
        Preserve:\n\
        1. The current goal\n\
        2. Important findings and decisions\n\
        3. Files read or changed\n\
        4. Remaining work\n\
        5. User constraints and preferences\n\
        Be compact but concrete.\n\n\
        {truncated}"
        );

        let request = CreateMessageParams::new(RequiredMessageParams {
            model: MODEL.to_string(),
            messages: vec![Message::new_text(Role::User, prompt)],
            max_tokens: 2000,
        });

        let response = self.client.create_message(Some(&request)).await?;
        Ok(response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn remember_recent_file(&mut self, path: &str) {
        self.compact_state.recent_files.retain(|p| p != path);
        self.compact_state.recent_files.push(path.to_string());

        if self.compact_state.recent_files.len() > 5 {
            let overflow = self.compact_state.recent_files.len() - 5;
            self.compact_state.recent_files.drain(0..overflow);
        }
    }
}

fn collect_tool_result_positions(messages: &[Message]) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();

    for (message_idx, message) in messages.iter().enumerate() {
        if !matches!(message.role, Role::User) {
            continue;
        }

        let MessageContent::Blocks { content } = &message.content else {
            continue;
        };

        for (block_idx, block) in content.iter().enumerate() {
            if matches!(block, ContentBlock::ToolResult { .. }) {
                positions.push((message_idx, block_idx));
            }
        }
    }

    positions
}
