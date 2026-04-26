use anyhow::{Context, Result};
use inquire::Select;
use serde_json::Value;
use std::fmt;
use strum_macros::{Display, EnumString};

const READ_PREFIXES: &[&str] = &["read", "list", "get", "show", "search", "query", "inspect"];
const HIGH_PREFIXES: &[&str] = &["delete", "remove", "drop", "shutdown"];
const HIGH_BASH_PATTERNS: &[&str] = &["rm -rf", "sudo", "shutdown", "reboot"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum CapabilitySource {
    Native,
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum CapabilityRisk {
    Read,
    Write,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityIntent {
    pub source: CapabilitySource,
    pub server: Option<String>,
    pub tool: String,
    pub risk: CapabilityRisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum PermissionMode {
    Default,
    Plan,
    Auto,
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            PermissionMode::Default => "default - ask for writes",
            PermissionMode::Plan => "plan - read only",
            PermissionMode::Auto => "auto - allow non-high operations",
        };

        write!(f, "{label}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecision {
    pub behavior: PermissionBehavior,
    pub reason: String,
}

impl PermissionDecision {
    fn allow(reason: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Allow,
            reason: reason.into(),
        }
    }

    fn ask(reason: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Ask,
            reason: reason.into(),
        }
    }

    fn deny(reason: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Deny,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
enum UserPermissionChoice {
    #[strum(serialize = "allow once")]
    AllowOnce,
    #[strum(serialize = "deny")]
    Deny,
    #[strum(serialize = "always allow this tool")]
    AlwaysAllow,
}

#[derive(Debug)]
pub struct PermissionManager {
    mode: PermissionMode,
    always_allowed_tools: Vec<String>,
    consecutive_denials: usize,
    max_consecutive_denials: usize,
}

impl PermissionManager {
    pub fn try_new(mode: PermissionMode) -> Result<Self> {
        Ok(Self {
            mode,
            always_allowed_tools: vec!["read_file".to_string()],
            consecutive_denials: 0,
            max_consecutive_denials: 3,
        })
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
    }

    pub fn rules(&self) -> &[String] {
        &self.always_allowed_tools
    }

    pub fn check(&mut self, tool_name: &str, tool_input: &Value) -> PermissionDecision {
        let intent = normalize_capability(tool_name, tool_input);

        if intent.risk == CapabilityRisk::Read {
            self.consecutive_denials = 0;
            return PermissionDecision::allow("Read-only capability allowed");
        }

        if self.mode == PermissionMode::Plan {
            return PermissionDecision::deny("Plan mode: write operations are blocked");
        }

        if intent.risk == CapabilityRisk::High {
            return PermissionDecision::ask(format!(
                "High-risk capability requires approval: {}",
                intent.tool
            ));
        }

        if self.is_always_allowed(tool_name) {
            self.consecutive_denials = 0;
            return PermissionDecision::allow(format!("Always allowed tool: {tool_name}"));
        }

        match self.mode {
            PermissionMode::Auto => {
                self.consecutive_denials = 0;
                PermissionDecision::allow("Auto mode: non-high capability auto-approved")
            }
            PermissionMode::Default | PermissionMode::Plan => {
                PermissionDecision::ask(format!("Default mode: asking user for {tool_name}"))
            }
        }
    }

    pub fn ask_user(&mut self, tool_name: &str, tool_input: &Value) -> Result<bool> {
        let preview = truncate_for_prompt(tool_input, 200);
        let prompt = format!("[Permission] {tool_name}: {preview}");

        let choice = Select::new(
            &prompt,
            vec![
                UserPermissionChoice::AllowOnce,
                UserPermissionChoice::Deny,
                UserPermissionChoice::AlwaysAllow,
            ],
        )
        .prompt()
        .context("failed to read permission decision")?;

        let approved = self.apply_user_choice(choice, tool_name);
        if !approved && self.should_suggest_plan_mode() {
            println!(
                "[{} consecutive denials -- consider switching to plan mode]",
                self.consecutive_denials
            );
        }

        Ok(approved)
    }

    fn apply_user_choice(&mut self, choice: UserPermissionChoice, tool_name: &str) -> bool {
        match choice {
            UserPermissionChoice::AllowOnce => {
                self.consecutive_denials = 0;
                true
            }
            UserPermissionChoice::Deny => {
                self.consecutive_denials += 1;
                false
            }
            UserPermissionChoice::AlwaysAllow => {
                self.allow_tool(tool_name);
                self.consecutive_denials = 0;
                true
            }
        }
    }

    fn allow_tool(&mut self, tool_name: &str) {
        if !self.is_always_allowed(tool_name) {
            self.always_allowed_tools.push(tool_name.to_string());
        }
    }

    fn is_always_allowed(&self, tool_name: &str) -> bool {
        self.always_allowed_tools
            .iter()
            .any(|allowed| allowed == tool_name)
    }

    fn should_suggest_plan_mode(&self) -> bool {
        self.consecutive_denials >= self.max_consecutive_denials
    }
}

pub fn normalize_capability(tool_name: &str, tool_input: &Value) -> CapabilityIntent {
    let (source, server, short_tool) = parse_source(tool_name);
    let risk = classify_risk(&short_tool, tool_input);

    CapabilityIntent {
        source,
        server,
        tool: short_tool,
        risk,
    }
}

fn parse_source(tool_name: &str) -> (CapabilitySource, Option<String>, String) {
    if let Some(rest) = tool_name.strip_prefix("mcp__")
        && let Some((server, tool)) = rest.rsplit_once("__")
        && !server.is_empty()
        && !tool.is_empty()
    {
        return (
            CapabilitySource::Mcp,
            Some(server.to_string()),
            tool.to_string(),
        );
    }

    (CapabilitySource::Native, None, tool_name.to_string())
}

fn classify_risk(tool_name: &str, tool_input: &Value) -> CapabilityRisk {
    if tool_name == "read_file" || starts_with_any(tool_name, READ_PREFIXES) {
        return CapabilityRisk::Read;
    }

    if tool_name == "bash" {
        let command = tool_input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();

        return if HIGH_BASH_PATTERNS
            .iter()
            .any(|pattern| command.contains(pattern))
        {
            CapabilityRisk::High
        } else {
            CapabilityRisk::Write
        };
    }

    if starts_with_any(tool_name, HIGH_PREFIXES) {
        CapabilityRisk::High
    } else {
        CapabilityRisk::Write
    }
}

fn starts_with_any(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn truncate_for_prompt(value: &Value, limit: usize) -> String {
    let text = value.to_string();
    if text.chars().count() <= limit {
        return text;
    }

    let truncated = text.chars().take(limit).collect::<String>();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CapabilityRisk, CapabilitySource, PermissionBehavior, PermissionManager, PermissionMode,
        UserPermissionChoice, normalize_capability,
    };

    #[test]
    fn normalizes_mcp_tool_intent() {
        let intent = normalize_capability("mcp__demo__db__query", &json!({ "sql": "select 1" }));

        assert_eq!(intent.source, CapabilitySource::Mcp);
        assert_eq!(intent.server.as_deref(), Some("demo__db"));
        assert_eq!(intent.tool, "query");
        assert_eq!(intent.risk, CapabilityRisk::Read);
    }

    #[test]
    fn normalizes_high_risk_native_intent() {
        let intent = normalize_capability("delete_file", &json!({ "path": "a.txt" }));

        assert_eq!(intent.source, CapabilitySource::Native);
        assert_eq!(intent.server, None);
        assert_eq!(intent.tool, "delete_file");
        assert_eq!(intent.risk, CapabilityRisk::High);
    }

    #[test]
    fn classifies_high_risk_bash() {
        let intent = normalize_capability("bash", &json!({ "command": "sudo ls" }));

        assert_eq!(intent.risk, CapabilityRisk::High);
    }

    #[test]
    fn read_capabilities_are_allowed_in_all_modes() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::Plan,
            PermissionMode::Auto,
        ] {
            let mut manager = PermissionManager::try_new(mode).unwrap();
            let decision = manager.check("read_file", &json!({ "path": "src/main.rs" }));

            assert_eq!(decision.behavior, PermissionBehavior::Allow);
        }
    }

    #[test]
    fn plan_mode_blocks_non_read_tools() {
        let mut manager = PermissionManager::try_new(PermissionMode::Plan).unwrap();

        let decision = manager.check("write_file", &json!({ "path": "a.txt", "content": "x" }));

        assert_eq!(decision.behavior, PermissionBehavior::Deny);
        assert!(decision.reason.contains("Plan mode"));
    }

    #[test]
    fn high_risk_tools_require_approval() {
        let mut manager = PermissionManager::try_new(PermissionMode::Auto).unwrap();

        let decision = manager.check("bash", &json!({ "command": "sudo ls" }));

        assert_eq!(decision.behavior, PermissionBehavior::Ask);
        assert!(decision.reason.contains("High-risk"));
    }

    #[test]
    fn auto_mode_allows_non_high_tools() {
        let mut manager = PermissionManager::try_new(PermissionMode::Auto).unwrap();

        let decision = manager.check("write_file", &json!({ "path": "a.txt", "content": "x" }));

        assert_eq!(decision.behavior, PermissionBehavior::Allow);
        assert!(decision.reason.contains("Auto mode"));
    }

    #[test]
    fn default_mode_asks_for_non_high_writes() {
        let mut manager = PermissionManager::try_new(PermissionMode::Default).unwrap();

        let decision = manager.check("edit_file", &json!({ "path": "src/lib.rs" }));

        assert_eq!(decision.behavior, PermissionBehavior::Ask);
        assert!(decision.reason.contains("asking user"));
    }

    #[test]
    fn always_allow_adds_exact_tool_allowlist_entry() {
        let mut manager = PermissionManager::try_new(PermissionMode::Default).unwrap();

        let approved = manager.apply_user_choice(UserPermissionChoice::AlwaysAllow, "edit_file");

        assert!(approved);
        assert!(manager.rules().contains(&"edit_file".to_string()));
        let decision = manager.check("edit_file", &json!({ "path": "src/lib.rs" }));
        assert_eq!(decision.behavior, PermissionBehavior::Allow);
    }

    #[test]
    fn high_risk_still_asks_even_when_tool_was_always_allowed() {
        let mut manager = PermissionManager::try_new(PermissionMode::Default).unwrap();
        manager.apply_user_choice(UserPermissionChoice::AlwaysAllow, "bash");

        let decision = manager.check("bash", &json!({ "command": "sudo ls" }));

        assert_eq!(decision.behavior, PermissionBehavior::Ask);
    }

    #[test]
    fn suggests_plan_mode_after_repeated_denials() {
        let mut manager = PermissionManager::try_new(PermissionMode::Default).unwrap();

        for _ in 0..manager.max_consecutive_denials {
            let approved = manager.apply_user_choice(UserPermissionChoice::Deny, "bash");
            assert!(!approved);
        }

        assert!(manager.should_suggest_plan_mode());
    }
}
