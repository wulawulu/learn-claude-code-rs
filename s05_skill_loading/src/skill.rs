use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;
use walkdir::WalkDir;

pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

pub struct SkillDocument {
    pub manifest: SkillManifest,
    pub body: String,
}

impl std::fmt::Display for SkillDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r#"<skill name="{}">
{}
</skill>"#,
            self.manifest.name, self.body
        )
    }
}

pub fn get_skill_registry(skills_dir: PathBuf) -> Result<SkillRegistry> {
    let mut registry = SkillRegistry::new(skills_dir);
    registry.load_skills()?;
    Ok(registry)
}

pub struct SkillRegistry {
    skills_dir: PathBuf,
    skills: HashMap<String, SkillDocument>,
}

impl SkillRegistry {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            skills: HashMap::new(),
        }
    }

    pub fn load_skills(&mut self) -> Result<()> {
        self.skills.clear();

        if !self.skills_dir.exists() {
            return Ok(());
        }

        for entry in WalkDir::new(&self.skills_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| entry.file_name().to_str() == Some("SKILL.md"))
        {
            let path = entry.path();

            let content = std::fs::read_to_string(path)
                .with_context(|| format!("can't read skill file: {}", path.display()))?;

            let (meta, body) = parse_frontmatter(&content);
            let fallback_name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let name = meta.name.unwrap_or(fallback_name);
            let description = meta
                .description
                .unwrap_or_else(|| "No description".to_string());

            let document = SkillDocument {
                manifest: SkillManifest {
                    name: name.clone(),
                    description,
                    path: path.to_path_buf(),
                },
                body,
            };

            self.skills.insert(name, document);
        }

        Ok(())
    }

    pub fn describe_available(&self) -> String {
        if self.skills.is_empty() {
            return "(no skills available)".to_string();
        }

        let mut names = self.skills.keys().cloned().collect::<Vec<_>>();
        names.sort();

        names
            .into_iter()
            .filter_map(|name| {
                self.skills.get(&name).map(|skill| {
                    format!(" - {}: {}", skill.manifest.name, skill.manifest.description)
                })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn load_full_text(&self, name: &str) -> String {
        match self.skills.get(name) {
            Some(skill) => skill.to_string(),
            None => {
                let mut names = self.skills.keys().cloned().collect::<Vec<_>>();
                names.sort();
                format!(
                    "Error: Unknown skill '{}'. Available: {}",
                    name,
                    names.join(", ")
                )
            }
        }
    }

    pub fn skills(&self) -> &HashMap<String, SkillDocument> {
        &self.skills
    }
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

fn parse_frontmatter(text: &str) -> (SkillFrontmatter, String) {
    let text = text.replace("\r\n", "\n");

    let Some(rest) = text.strip_prefix("---\n") else {
        return (SkillFrontmatter::default(), text.trim().to_string());
    };

    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        return (SkillFrontmatter::default(), text.trim().to_string());
    };

    let meta = serde_yaml::from_str::<SkillFrontmatter>(frontmatter).unwrap_or_default();

    (meta, body.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_frontmatter;

    #[test]
    fn parses_frontmatter_with_lf_line_endings() {
        let input = "---\nname: test\ndescription: hello\n---\n\nbody";
        let (meta, body) = parse_frontmatter(input);

        assert_eq!(meta.name.as_deref(), Some("test"));
        assert_eq!(meta.description.as_deref(), Some("hello"));
        assert_eq!(body, "body");
    }

    #[test]
    fn parses_frontmatter_with_crlf_line_endings() {
        let input = "---\r\nname: test\r\ndescription: hello\r\n---\r\n\r\nbody";
        let (meta, body) = parse_frontmatter(input);

        assert_eq!(meta.name.as_deref(), Some("test"));
        assert_eq!(meta.description.as_deref(), Some("hello"));
        assert_eq!(body, "body");
    }
}
