use anyhow::{Context, Result};
use inquire::Select;
use regex::Regex;
use serde_json::Value;
use std::fmt;
use strum_macros::EnumString;
use wildmatch::WildMatch;

pub const READ_ONLY_TOOLS: &[&str] = &["read_file", "bash_readonly"];
pub const WRITE_TOOLS: &[&str] = &["write_file", "edit_file", "bash"];

#[derive(Debug)]
pub struct ValidationRule {
    pub name: &'static str,
    pub pattern: &'static str,
    pub regex: Regex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFailure {
    pub name: &'static str,
    pub pattern: &'static str,
}

#[derive(Debug, Default)]
pub struct BashSecurityValidator {
    validators: Vec<ValidationRule>,
}

impl BashSecurityValidator {
    pub fn try_new() -> Result<Self> {
        Ok(Self {
            validators: vec![
                ValidationRule {
                    name: "shell_metachar",
                    pattern: r"[;&|`$]",
                    regex: Regex::new(r"[;&|`$]")
                        .context("failed to compile shell_metachar regex")?,
                },
                ValidationRule {
                    name: "sudo",
                    pattern: r"\bsudo\b",
                    regex: Regex::new(r"\bsudo\b").context("failed to compile sudo regex")?,
                },
                ValidationRule {
                    name: "rm_rf",
                    pattern: r"\brm\s+(-[a-zA-Z]*)?r",
                    regex: Regex::new(r"\brm\s+(-[a-zA-Z]*)?r")
                        .context("failed to compile rm_rf regex")?,
                },
                ValidationRule {
                    name: "cmd_substitution",
                    pattern: r"\$\(",
                    regex: Regex::new(r"\$\(")
                        .context("failed to compile cmd_substitution regex")?,
                },
                ValidationRule {
                    name: "ifs_injection",
                    pattern: r"\bIFS\s*=",
                    regex: Regex::new(r"\bIFS\s*=")
                        .context("failed to compile ifs_injection regex")?,
                },
            ],
        })
    }

    pub fn validate(&self, command: &str) -> Vec<ValidationFailure> {
        self.validators
            .iter()
            .filter(|rule| rule.regex.is_match(command))
            .map(|rule| ValidationFailure {
                name: rule.name,
                pattern: rule.pattern,
            })
            .collect()
    }

    pub fn is_safe(&self, command: &str) -> bool {
        self.validate(command).is_empty()
    }

    pub fn describe_failures(&self, command: &str) -> String {
        let failures = self.validate(command);
        if failures.is_empty() {
            return "No issues detected".to_string();
        }

        let parts = failures
            .iter()
            .map(|failure| format!("{} (pattern: {})", failure.name, failure.pattern))
            .collect::<Vec<_>>()
            .join(", ");

        format!("Security flags: {parts}")
    }
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
            PermissionMode::Default => "default - ask by default",
            PermissionMode::Plan => "plan - read only",
            PermissionMode::Auto => "auto - allow reads, ask for writes",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserPermissionChoice {
    AllowOnce,
    Deny,
    AlwaysAllow,
}

impl fmt::Display for UserPermissionChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            UserPermissionChoice::AllowOnce => "allow once",
            UserPermissionChoice::Deny => "deny",
            UserPermissionChoice::AlwaysAllow => "always allow this tool",
        };

        write!(f, "{label}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRule {
    pub tool: String,
    pub path: Option<String>,
    pub content: Option<String>,
    pub behavior: PermissionBehavior,
}

impl PermissionRule {
    pub fn allow_tool(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            path: None,
            content: None,
            behavior: PermissionBehavior::Allow,
        }
    }

    pub fn deny_tool_content(tool: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            path: None,
            content: Some(content.into()),
            behavior: PermissionBehavior::Deny,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    fn matches(&self, tool_name: &str, tool_input: &Value) -> bool {
        if self.tool != "*" && self.tool != tool_name {
            return false;
        }

        if let Some(path_pattern) = &self.path {
            let path = tool_input.get("path").and_then(Value::as_str).unwrap_or("");
            if !WildMatch::new(path_pattern).matches(path) {
                return false;
            }
        }

        if let Some(content_pattern) = &self.content {
            let command = tool_input
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !WildMatch::new(content_pattern).matches(command) {
                return false;
            }
        }

        true
    }
}

impl fmt::Display for PermissionRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tool={}", self.tool)?;

        if let Some(path) = &self.path {
            write!(f, ", path={path}")?;
        }

        if let Some(content) = &self.content {
            write!(f, ", content={content}")?;
        }

        let behavior = match self.behavior {
            PermissionBehavior::Allow => "allow",
            PermissionBehavior::Deny => "deny",
            PermissionBehavior::Ask => "ask",
        };
        write!(f, ", behavior={behavior}")
    }
}

#[derive(Debug)]
pub struct PermissionManager {
    mode: PermissionMode,
    rules: Vec<PermissionRule>,
    bash_validator: BashSecurityValidator,
    consecutive_denials: usize,
    max_consecutive_denials: usize,
}

impl PermissionManager {
    pub fn try_new(mode: PermissionMode) -> Result<Self> {
        Self::try_new_with_rules(mode, default_rules())
    }

    pub fn try_new_with_rules(mode: PermissionMode, rules: Vec<PermissionRule>) -> Result<Self> {
        Ok(Self {
            mode,
            rules,
            bash_validator: BashSecurityValidator::try_new()?,
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

    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    pub fn check(&mut self, tool_name: &str, tool_input: &Value) -> PermissionDecision {
        if let Some(decision) = self.check_bash_security(tool_name, tool_input) {
            return decision;
        }

        if let Some(decision) = self.match_rule(tool_name, tool_input, PermissionBehavior::Deny) {
            return decision;
        }

        if let Some(decision) = self.check_mode(tool_name) {
            return decision;
        }

        if let Some(decision) = self.match_rule(tool_name, tool_input, PermissionBehavior::Allow) {
            self.consecutive_denials = 0;
            return decision;
        }

        PermissionDecision {
            behavior: PermissionBehavior::Ask,
            reason: format!("No rule matched for {tool_name}, asking user"),
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

    fn check_bash_security(
        &self,
        tool_name: &str,
        tool_input: &Value,
    ) -> Option<PermissionDecision> {
        if tool_name != "bash" {
            return None;
        }

        let command = tool_input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("");
        let failures = self.bash_validator.validate(command);
        if failures.is_empty() {
            return None;
        }

        let severe_rules = ["sudo", "rm_rf"];
        let has_severe_hit = failures
            .iter()
            .any(|failure| severe_rules.contains(&failure.name));
        let summary = self.bash_validator.describe_failures(command);

        Some(if has_severe_hit {
            PermissionDecision {
                behavior: PermissionBehavior::Deny,
                reason: format!("Bash validator: {summary}"),
            }
        } else {
            PermissionDecision {
                behavior: PermissionBehavior::Ask,
                reason: format!("Bash validator flagged: {summary}"),
            }
        })
    }

    fn match_rule(
        &self,
        tool_name: &str,
        tool_input: &Value,
        behavior: PermissionBehavior,
    ) -> Option<PermissionDecision> {
        self.rules
            .iter()
            .find(|rule| rule.behavior == behavior && rule.matches(tool_name, tool_input))
            .map(|rule| PermissionDecision {
                behavior,
                reason: match behavior {
                    PermissionBehavior::Allow => format!("Matched allow rule: {rule:?}"),
                    PermissionBehavior::Deny => format!("Blocked by deny rule: {rule:?}"),
                    PermissionBehavior::Ask => format!("Matched ask rule: {rule:?}"),
                },
            })
    }

    fn check_mode(&self, tool_name: &str) -> Option<PermissionDecision> {
        match self.mode {
            PermissionMode::Default => None,
            PermissionMode::Plan => {
                if is_write_tool(tool_name) {
                    Some(PermissionDecision {
                        behavior: PermissionBehavior::Deny,
                        reason: "Plan mode: write operations are blocked".to_string(),
                    })
                } else {
                    Some(PermissionDecision {
                        behavior: PermissionBehavior::Allow,
                        reason: "Plan mode: read-only allowed".to_string(),
                    })
                }
            }
            PermissionMode::Auto => {
                if is_read_only_tool(tool_name) || tool_name == "read_file" {
                    Some(PermissionDecision {
                        behavior: PermissionBehavior::Allow,
                        reason: "Auto mode: read-only tool auto-approved".to_string(),
                    })
                } else {
                    None
                }
            }
        }
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
                self.rules
                    .push(PermissionRule::allow_tool(tool_name).with_path("*"));
                self.consecutive_denials = 0;
                true
            }
        }
    }

    fn should_suggest_plan_mode(&self) -> bool {
        self.consecutive_denials >= self.max_consecutive_denials
    }
}

pub fn default_rules() -> Vec<PermissionRule> {
    vec![
        PermissionRule::deny_tool_content("bash", "rm -rf /"),
        PermissionRule::deny_tool_content("bash", "sudo *"),
        PermissionRule::allow_tool("read_file").with_path("*"),
    ]
}

fn is_read_only_tool(tool_name: &str) -> bool {
    READ_ONLY_TOOLS.contains(&tool_name)
}

fn is_write_tool(tool_name: &str) -> bool {
    WRITE_TOOLS.contains(&tool_name)
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
        BashSecurityValidator, PermissionBehavior, PermissionManager, PermissionMode,
        PermissionRule, UserPermissionChoice,
    };

    #[test]
    fn safe_command_passes_validation() {
        let validator = BashSecurityValidator::try_new().unwrap();

        assert!(validator.validate("ls -la src").is_empty());
        assert!(validator.is_safe("cargo test"));
        assert_eq!(validator.describe_failures("pwd"), "No issues detected");
    }

    #[test]
    fn flags_shell_metacharacters() {
        let validator = BashSecurityValidator::try_new().unwrap();

        let failures = validator.validate("cat Cargo.toml | head");
        assert!(failures.iter().any(|f| f.name == "shell_metachar"));
    }

    #[test]
    fn flags_sudo_usage() {
        let validator = BashSecurityValidator::try_new().unwrap();

        let failures = validator.validate("sudo apt install ripgrep");
        assert!(failures.iter().any(|f| f.name == "sudo"));
    }

    #[test]
    fn flags_recursive_rm() {
        let validator = BashSecurityValidator::try_new().unwrap();

        let failures = validator.validate("rm -rf target");
        assert!(failures.iter().any(|f| f.name == "rm_rf"));
    }

    #[test]
    fn flags_command_substitution() {
        let validator = BashSecurityValidator::try_new().unwrap();

        let failures = validator.validate("echo $(whoami)");
        assert!(failures.iter().any(|f| f.name == "shell_metachar"));
        assert!(failures.iter().any(|f| f.name == "cmd_substitution"));
    }

    #[test]
    fn flags_ifs_assignment() {
        let validator = BashSecurityValidator::try_new().unwrap();

        let failures = validator.validate("IFS=, read -ra parts <<< \"$input\"");
        assert!(failures.iter().any(|f| f.name == "ifs_injection"));
    }

    #[test]
    fn describe_failures_lists_all_hits() {
        let validator = BashSecurityValidator::try_new().unwrap();

        let description = validator.describe_failures("sudo rm -rf /tmp/test");
        assert!(description.contains("sudo"));
        assert!(description.contains("rm_rf"));
    }

    #[test]
    fn deny_rules_match_before_mode_logic() {
        let mut manager = PermissionManager::try_new(PermissionMode::Default).unwrap();

        let decision = manager.check("bash", &json!({ "command": "sudo ls" }));
        assert_eq!(decision.behavior, PermissionBehavior::Deny);
        assert!(decision.reason.contains("Bash validator"));
    }

    #[test]
    fn bash_validator_can_escalate_to_ask() {
        let mut manager = PermissionManager::try_new(PermissionMode::Default).unwrap();

        let decision = manager.check("bash", &json!({ "command": "cat Cargo.toml | head" }));
        assert_eq!(decision.behavior, PermissionBehavior::Ask);
        assert!(decision.reason.contains("Bash validator flagged"));
    }

    #[test]
    fn plan_mode_blocks_write_tools() {
        let mut manager = PermissionManager::try_new(PermissionMode::Plan).unwrap();

        let decision = manager.check("write_file", &json!({ "path": "a.txt", "content": "x" }));
        assert_eq!(decision.behavior, PermissionBehavior::Deny);
        assert!(decision.reason.contains("Plan mode"));
    }

    #[test]
    fn plan_mode_allows_reads() {
        let mut manager = PermissionManager::try_new(PermissionMode::Plan).unwrap();

        let decision = manager.check("read_file", &json!({ "path": "src/main.rs" }));
        assert_eq!(decision.behavior, PermissionBehavior::Allow);
    }

    #[test]
    fn auto_mode_allows_read_only_tools() {
        let mut manager = PermissionManager::try_new(PermissionMode::Auto).unwrap();

        let decision = manager.check("read_file", &json!({ "path": "src/main.rs" }));
        assert_eq!(decision.behavior, PermissionBehavior::Allow);
        assert!(decision.reason.contains("Auto mode"));
    }

    #[test]
    fn allow_rules_apply_after_mode_checks() {
        let rules = vec![PermissionRule::allow_tool("edit_file").with_path("src/*")];
        let mut manager =
            PermissionManager::try_new_with_rules(PermissionMode::Default, rules).unwrap();

        let decision = manager.check(
            "edit_file",
            &json!({ "path": "src/lib.rs", "old_text": "a", "new_text": "b" }),
        );
        assert_eq!(decision.behavior, PermissionBehavior::Allow);
        assert!(decision.reason.contains("Matched allow rule"));
    }

    #[test]
    fn unmatched_tool_falls_back_to_ask() {
        let mut manager =
            PermissionManager::try_new_with_rules(PermissionMode::Default, Vec::new()).unwrap();

        let decision = manager.check("edit_file", &json!({ "path": "src/lib.rs" }));
        assert_eq!(decision.behavior, PermissionBehavior::Ask);
        assert!(decision.reason.contains("asking user"));
    }

    #[test]
    fn tracks_consecutive_denials() {
        let mut manager =
            PermissionManager::try_new_with_rules(PermissionMode::Default, Vec::new()).unwrap();

        manager.consecutive_denials += 1;
        manager.consecutive_denials += 1;
        assert_eq!(manager.consecutive_denials, 2);

        manager.consecutive_denials = 0;
        assert_eq!(manager.consecutive_denials, 0);
    }

    #[test]
    fn always_allow_adds_wildcard_rule() {
        let mut manager =
            PermissionManager::try_new_with_rules(PermissionMode::Default, Vec::new()).unwrap();

        let approved = manager.apply_user_choice(UserPermissionChoice::AlwaysAllow, "read_file");

        assert!(approved);
        assert_eq!(manager.rules.len(), 1);
        assert_eq!(manager.rules[0].tool, "read_file");
        assert_eq!(manager.rules[0].path.as_deref(), Some("*"));
        assert_eq!(manager.rules[0].behavior, PermissionBehavior::Allow);
    }

    #[test]
    fn suggests_plan_mode_after_repeated_denials() {
        let mut manager =
            PermissionManager::try_new_with_rules(PermissionMode::Default, Vec::new()).unwrap();

        for _ in 0..manager.max_consecutive_denials {
            let approved = manager.apply_user_choice(UserPermissionChoice::Deny, "bash");
            assert!(!approved);
        }

        assert!(manager.should_suggest_plan_mode());
    }

    #[test]
    fn content_rules_use_simple_glob_matching() {
        let rules = vec![PermissionRule::deny_tool_content("bash", "sudo *")];
        let mut manager =
            PermissionManager::try_new_with_rules(PermissionMode::Default, rules).unwrap();

        let decision = manager.check("bash", &json!({ "command": "sudo apt update" }));
        assert_eq!(decision.behavior, PermissionBehavior::Deny);
    }
}
