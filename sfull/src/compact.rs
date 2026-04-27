use anthropic_ai_sdk::types::message::{ContentBlock, Message, MessageContent, Role};
use anyhow::Context as _;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const KEEP_RECENT_TOOL_RESULTS: usize = 3;
const PERSIST_THRESHOLD: usize = 30_000;
const PREVIEW_CHARS: usize = 2_000;
const TRANSCRIPT_DIR: &str = ".claude/transcripts";
const OUTPUT_DIR: &str = ".claude/tool-results";
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
        .unwrap_or_default()
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
    let preview = output.chars().take(PREVIEW_CHARS).collect::<String>();
    Ok(format!(
        "<persisted-output>\nFull output saved to: {}\nPreview:\n{}\n</persisted-output>",
        output_path.display(),
        preview
    ))
}

pub fn compacted_context(summary: String) -> Vec<Message> {
    vec![Message::new_text(
        Role::User,
        format!("This conversation was compacted so the agent can continue working.\n\n{summary}"),
    )]
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
