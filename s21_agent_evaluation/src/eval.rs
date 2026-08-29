use std::path::{Path, PathBuf};

use anthropic_ai_sdk::client::AnthropicClient;
use anyhow::{Context, Result};
use serde_json::Value;
use tempfile::{Builder, TempDir};
use tokio::process::Command;

use crate::Agent;
use crate::judge::LlmJudgeEvaluator;
use crate::tool::{ToolContext, toolset};

#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub name: String,
    pub input: Value,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct AgentRun {
    pub final_answer: String,
    pub tool_calls: Vec<ToolCallRecord>,
}

#[derive(Debug, Clone)]
pub struct EvalResult {
    pub passed: bool,
    pub score: f32,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct EvalCase {
    pub name: String,
    pub prompt: String,
    pub fixture: Option<PathBuf>,
    pub evaluator: Evaluator,
}

#[derive(Debug, Clone)]
pub enum Evaluator {
    Assertion(AssertionEvaluator),
    LlmJudge(LlmJudgeEvaluator),
}

#[derive(Debug, Clone)]
pub enum Assertion {
    CommandSucceeds {
        program: String,
        args: Vec<String>,
    },
    FinalAnswerContains {
        text: String,
    },
    ToolCallMatches {
        tool: String,
        input_contains: String,
    },
}

#[derive(Debug, Clone)]
pub struct AssertionEvaluator {
    pub assertions: Vec<Assertion>,
}

impl AssertionEvaluator {
    pub async fn evaluate(&self, run: &AgentRun, workspace: &Path) -> Result<EvalResult> {
        let mut passed_count = 0;
        let mut reasons = Vec::new();

        for assertion in &self.assertions {
            let (passed, reason) = assertion.check(run, workspace).await?;
            if passed {
                passed_count += 1;
            }
            reasons.push(reason);
        }

        let total = self.assertions.len();
        let score = if total == 0 {
            1.0
        } else {
            passed_count as f32 / total as f32
        };

        Ok(EvalResult {
            passed: passed_count == total,
            score,
            reason: reasons.join("\n"),
        })
    }
}

impl Assertion {
    async fn check(&self, run: &AgentRun, workspace: &Path) -> Result<(bool, String)> {
        match self {
            Assertion::CommandSucceeds { program, args } => {
                let output = Command::new(program)
                    .args(args)
                    .current_dir(workspace)
                    .output()
                    .await
                    .with_context(|| format!("failed to run evaluator command: {program}"))?;
                let command = std::iter::once(program.as_str())
                    .chain(args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                let passed = output.status.success();
                let reason = if passed {
                    format!("{command} passed")
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("{command} failed: {}", stderr.trim())
                };
                Ok((passed, reason))
            }
            Assertion::FinalAnswerContains { text } => {
                let passed = run.final_answer.contains(text);
                let reason = if passed {
                    format!("answer contains {text}")
                } else {
                    format!("answer does not contain {text}")
                };
                Ok((passed, reason))
            }
            Assertion::ToolCallMatches {
                tool,
                input_contains,
            } => {
                let passed = run.tool_calls.iter().any(|call| {
                    call.name == *tool && call.input.to_string().contains(input_contains)
                });
                let reason = if passed {
                    format!("agent called {tool} with input containing {input_contains}")
                } else {
                    format!("agent did not call {tool} with input containing {input_contains}")
                };
                Ok((passed, reason))
            }
        }
    }
}

pub struct PreparedWorkspace {
    directory: TempDir,
}

impl PreparedWorkspace {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

pub fn prepare_workspace(case: &EvalCase) -> Result<PreparedWorkspace> {
    let directory = Builder::new()
        .prefix(&format!("s21-{}-", case.name))
        .tempdir()
        .context("failed to create evaluation workspace")?;

    if let Some(fixture) = &case.fixture {
        copy_dir_recursive(fixture, directory.path())?;
    }

    Ok(PreparedWorkspace { directory })
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("failed to read fixture {}", source.display()))?
    {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub struct EvalRunner {
    client: AnthropicClient,
}

impl EvalRunner {
    pub fn new(client: AnthropicClient) -> Self {
        Self { client }
    }

    pub async fn run_case(&self, case: &EvalCase) -> Result<(AgentRun, EvalResult)> {
        println!("  preparing workspace...");
        let workspace = prepare_workspace(case)?;

        println!("  running agent...");
        let mut agent = Agent::new(
            self.client.clone(),
            ToolContext {
                work_dir: workspace.path().to_path_buf(),
            },
            toolset(),
        );
        let run = agent.run(&case.prompt).await?;

        for call in &run.tool_calls {
            println!("  tool: {} {}", call.name, call.input);
        }

        let result = match &case.evaluator {
            Evaluator::Assertion(evaluator) => {
                println!("  evaluating assertions...");
                evaluator.evaluate(&run, workspace.path()).await?
            }
            Evaluator::LlmJudge(evaluator) => {
                println!("  judging with LLM...");
                evaluator
                    .evaluate(&self.client, &case.prompt, &run.final_answer)
                    .await?
            }
        };

        Ok((run, result))
    }
}

pub fn default_cases() -> Vec<EvalCase> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    vec![
        EvalCase {
            name: "bug_fix".to_string(),
            prompt: "The calculator crate crashes when dividing by zero.\n\n\
                     Fix the bug without changing the public function signature.\n\
                     Run the tests before finishing."
                .to_string(),
            fixture: Some(fixtures.join("bug_fix")),
            evaluator: Evaluator::Assertion(AssertionEvaluator {
                assertions: vec![
                    Assertion::CommandSucceeds {
                        program: "cargo".to_string(),
                        args: vec!["test".to_string()],
                    },
                    Assertion::ToolCallMatches {
                        tool: "bash".to_string(),
                        input_contains: "cargo test".to_string(),
                    },
                ],
            }),
        },
        EvalCase {
            name: "file_lookup".to_string(),
            prompt: "What is the timeout configured for service-a?\n\n\
                     Find the answer from the files in the workspace. Use the read_file tool to inspect data.json."
                .to_string(),
            fixture: Some(fixtures.join("file_lookup")),
            evaluator: Evaluator::Assertion(AssertionEvaluator {
                assertions: vec![
                    Assertion::FinalAnswerContains {
                        text: "30".to_string(),
                    },
                    Assertion::ToolCallMatches {
                        tool: "read_file".to_string(),
                        input_contains: "data.json".to_string(),
                    },
                ],
            }),
        },
        EvalCase {
            name: "rust_explain".to_string(),
            prompt: r#"Explain to a Rust beginner why this code fails to compile.

Explain the borrowing problem and provide at least one valid fix.

```rust
fn main() {
    let mut values = vec![1, 2, 3];

    let first = &mut values[0];
    let second = &mut values[1];

    *first += 1;
    *second += 1;
}
```"#
                .to_string(),
            fixture: None,
            evaluator: Evaluator::LlmJudge(LlmJudgeEvaluator {
                rubric: vec![
                    "Must identify overlapping mutable borrows.".to_string(),
                    "Must explain that Rust does not allow overlapping &mut references."
                        .to_string(),
                    "Must provide at least one valid fix.".to_string(),
                    "Must not claim that Vec elements can never be mutated independently."
                        .to_string(),
                    "The explanation should be understandable to a Rust beginner.".to_string(),
                ],
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_case(fixture: Option<PathBuf>) -> EvalCase {
        EvalCase {
            name: "test".to_string(),
            prompt: String::new(),
            fixture,
            evaluator: Evaluator::Assertion(AssertionEvaluator { assertions: vec![] }),
        }
    }

    #[test]
    fn fixture_is_copied_and_original_is_not_modified() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("value.txt"), "original").unwrap();
        let case = empty_case(Some(fixture.path().to_path_buf()));

        let workspace = prepare_workspace(&case).unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("value.txt")).unwrap(),
            "original"
        );

        std::fs::write(workspace.path().join("value.txt"), "changed").unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.path().join("value.txt")).unwrap(),
            "original"
        );
    }

    #[tokio::test]
    async fn final_answer_contains_passes_and_fails() {
        let evaluator = AssertionEvaluator {
            assertions: vec![Assertion::FinalAnswerContains {
                text: "30".to_string(),
            }],
        };
        let workspace = tempfile::tempdir().unwrap();

        let passing = AgentRun {
            final_answer: "The timeout is 30 seconds.".to_string(),
            tool_calls: vec![],
        };
        assert!(
            evaluator
                .evaluate(&passing, workspace.path())
                .await
                .unwrap()
                .passed
        );

        let failing = AgentRun {
            final_answer: "The timeout is unknown.".to_string(),
            tool_calls: vec![],
        };
        assert!(
            !evaluator
                .evaluate(&failing, workspace.path())
                .await
                .unwrap()
                .passed
        );
    }

    #[tokio::test]
    async fn tool_call_assertion_matches_name_and_input() {
        let evaluator = AssertionEvaluator {
            assertions: vec![Assertion::ToolCallMatches {
                tool: "read_file".to_string(),
                input_contains: "data.json".to_string(),
            }],
        };
        let run = AgentRun {
            final_answer: String::new(),
            tool_calls: vec![ToolCallRecord {
                name: "read_file".to_string(),
                input: serde_json::json!({ "path": "data.json" }),
                output: "{}".to_string(),
            }],
        };
        let workspace = tempfile::tempdir().unwrap();

        assert!(
            evaluator
                .evaluate(&run, workspace.path())
                .await
                .unwrap()
                .passed
        );
    }
}
