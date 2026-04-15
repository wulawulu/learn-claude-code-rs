use std::{
    borrow::Cow,
    sync::{LazyLock, RwLock},
};

use anyhow::{Context as _, Result};
use tera::Tera;

use derive_builder::Builder;

#[derive(Clone, Debug, Builder)]
#[builder(setter(into, strip_option))]
pub struct SystemPrompt {
    /// The role the agent is expected to fulfil.
    #[builder(default)]
    role: Option<String>,

    /// Available skills summary injected from the skill registry.
    #[builder(default)]
    skills_available: Option<String>,

    /// Persistent memory prompt injected from the state.
    #[builder(default)]
    memory: Option<String>,

    /// CLAUDE.md instructions loaded from the current environment.
    #[builder(default)]
    claude_md: Option<String>,

    /// Dynamic context that should be refreshed each render.
    #[builder(default)]
    dynamic_context: Option<String>,

    /// Guidance for when memories should be saved or ignored.
    #[builder(default)]
    memory_guidance: Option<String>,

    /// Additional guidelines for the agent to follow
    #[builder(default, setter(custom))]
    guidelines: Vec<String>,
    /// Additional constraints
    #[builder(default, setter(custom))]
    constraints: Vec<String>,

    /// Optional additional raw markdown to append to the prompt
    ///
    /// For instance, if you would like to support an AGENTS.md file, add it here.
    #[builder(default)]
    additional: Option<String>,

    /// The template to use for the system prompt
    #[builder(default = default_prompt_template())]
    template: Prompt,
}

impl SystemPrompt {
    pub fn builder() -> SystemPromptBuilder {
        SystemPromptBuilder::default()
    }

    pub fn to_prompt(&self) -> Prompt {
        self.clone().into()
    }

    /// Adds a guideline to the guidelines list.
    pub fn with_added_guideline(&mut self, guideline: impl AsRef<str>) -> &mut Self {
        self.guidelines.push(guideline.as_ref().to_string());
        self
    }

    /// Adds a constraint to the constraints list.
    pub fn with_added_constraint(&mut self, constraint: impl AsRef<str>) -> &mut Self {
        self.constraints.push(constraint.as_ref().to_string());
        self
    }

    /// Overwrites all guidelines.
    pub fn with_guidelines<T: IntoIterator<Item = S>, S: AsRef<str>>(
        &mut self,
        guidelines: T,
    ) -> &mut Self {
        self.guidelines = guidelines
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        self
    }

    /// Overwrites all constraints.
    pub fn with_constraints<T: IntoIterator<Item = S>, S: AsRef<str>>(
        &mut self,
        constraints: T,
    ) -> &mut Self {
        self.constraints = constraints
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        self
    }

    /// Changes the role.
    pub fn with_role(&mut self, role: impl Into<String>) -> &mut Self {
        self.role = Some(role.into());
        self
    }

    /// Sets the skills available section.
    pub fn with_skills_available(&mut self, skills_available: impl Into<String>) -> &mut Self {
        self.skills_available = Some(skills_available.into());
        self
    }

    /// Sets the memory section.
    pub fn with_memory(&mut self, memory: impl Into<String>) -> &mut Self {
        self.memory = Some(memory.into());
        self
    }

    /// Sets the CLAUDE.md section.
    pub fn with_claude_md(&mut self, claude_md: impl Into<String>) -> &mut Self {
        self.claude_md = Some(claude_md.into());
        self
    }

    /// Sets the dynamic context section.
    pub fn with_dynamic_context(&mut self, dynamic_context: impl Into<String>) -> &mut Self {
        self.dynamic_context = Some(dynamic_context.into());
        self
    }

    /// Sets the memory guidance section.
    pub fn with_memory_guidance(&mut self, guidance: impl Into<String>) -> &mut Self {
        self.memory_guidance = Some(guidance.into());
        self
    }

    /// Sets the additional markdown field.
    pub fn with_additional(&mut self, additional: impl Into<String>) -> &mut Self {
        self.additional = Some(additional.into());
        self
    }

    /// Sets the template.
    pub fn with_template(&mut self, template: impl Into<Prompt>) -> &mut Self {
        self.template = template.into();
        self
    }
}

impl From<String> for SystemPrompt {
    fn from(text: String) -> Self {
        SystemPrompt {
            role: None,
            skills_available: None,
            memory: None,
            claude_md: None,
            dynamic_context: None,
            memory_guidance: None,
            guidelines: Vec::new(),
            constraints: Vec::new(),
            additional: None,
            template: text.into(),
        }
    }
}

impl From<&'static str> for SystemPrompt {
    fn from(text: &'static str) -> Self {
        SystemPrompt {
            role: None,
            skills_available: None,
            memory: None,
            claude_md: None,
            dynamic_context: None,
            memory_guidance: None,
            guidelines: Vec::new(),
            constraints: Vec::new(),
            additional: None,
            template: text.into(),
        }
    }
}

impl From<SystemPrompt> for SystemPromptBuilder {
    fn from(val: SystemPrompt) -> Self {
        SystemPromptBuilder {
            role: Some(val.role),
            skills_available: Some(val.skills_available),
            memory: Some(val.memory),
            claude_md: Some(val.claude_md),
            dynamic_context: Some(val.dynamic_context),
            memory_guidance: Some(val.memory_guidance),
            guidelines: Some(val.guidelines),
            constraints: Some(val.constraints),
            additional: Some(val.additional),
            template: Some(val.template),
        }
    }
}

impl From<Prompt> for SystemPrompt {
    fn from(prompt: Prompt) -> Self {
        SystemPrompt {
            role: None,
            skills_available: None,
            memory: None,
            claude_md: None,
            dynamic_context: None,
            memory_guidance: None,
            guidelines: Vec::new(),
            constraints: Vec::new(),
            additional: None,
            template: prompt,
        }
    }
}

impl Default for SystemPrompt {
    fn default() -> Self {
        SystemPrompt {
            role: None,
            skills_available: None,
            memory: None,
            claude_md: None,
            dynamic_context: None,
            memory_guidance: None,
            guidelines: Vec::new(),
            constraints: Vec::new(),
            additional: None,
            template: default_prompt_template(),
        }
    }
}

impl SystemPromptBuilder {
    pub fn add_guideline(&mut self, guideline: &str) -> &mut Self {
        self.guidelines
            .get_or_insert_with(Vec::new)
            .push(guideline.to_string());
        self
    }

    pub fn add_constraint(&mut self, constraint: &str) -> &mut Self {
        self.constraints
            .get_or_insert_with(Vec::new)
            .push(constraint.to_string());
        self
    }

    pub fn guidelines<T: IntoIterator<Item = S>, S: AsRef<str>>(
        &mut self,
        guidelines: T,
    ) -> &mut Self {
        self.guidelines = Some(
            guidelines
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
        );
        self
    }

    pub fn constraints<T: IntoIterator<Item = S>, S: AsRef<str>>(
        &mut self,
        constraints: T,
    ) -> &mut Self {
        self.constraints = Some(
            constraints
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
        );
        self
    }
}

fn default_prompt_template() -> Prompt {
    include_str!("system_prompt_template.md").into()
}

#[allow(clippy::from_over_into)]
impl Into<Prompt> for SystemPrompt {
    fn into(self) -> Prompt {
        let SystemPrompt {
            role,
            skills_available,
            memory,
            claude_md,
            dynamic_context,
            memory_guidance,
            guidelines,
            constraints,
            template,
            additional,
        } = self;

        template
            .with_context_value("role", role)
            .with_context_value("skills_available", skills_available)
            .with_context_value("memory", memory)
            .with_context_value("claude_md", claude_md)
            .with_context_value("dynamic_context", dynamic_context)
            .with_context_value("memory_guidance", memory_guidance)
            .with_context_value("guidelines", guidelines)
            .with_context_value("constraints", constraints)
            .with_context_value("additional", additional)
    }
}

/// A Prompt can be used with large language models to prompt.
#[derive(Clone, Debug)]
pub struct Prompt {
    template_ref: TemplateRef,
    context: Option<tera::Context>,
}

/// References a to be rendered template
/// Either a one-off template or a tera template
#[derive(Clone, Debug)]
enum TemplateRef {
    OneOff(Cow<'static, str>),
    Tera(Cow<'static, str>),
}

pub static TERA: LazyLock<RwLock<Tera>> = LazyLock::new(|| RwLock::new(Tera::default()));

impl Prompt {
    /// Extend the repository with another Tera instance.
    ///
    /// You can use this to add your own templates, functions and partials.
    ///
    /// # Panics
    ///
    /// Panics if the `RWLock` is poisoned.
    ///
    /// # Errors
    ///
    /// Errors if the `Tera` instance cannot be extended.
    pub fn extend(other: &Tera) -> Result<()> {
        let mut swiftide_tera = TERA.write().unwrap();
        swiftide_tera.extend(other)?;
        Ok(())
    }

    /// Create a new prompt from a compiled template that is present in the Tera repository
    pub fn from_compiled_template(name: impl Into<Cow<'static, str>>) -> Prompt {
        Prompt {
            template_ref: TemplateRef::Tera(name.into()),
            context: None,
        }
    }

    /// Adds anything that implements [Into<tera::Context>], like `Serialize` to the Prompt
    #[must_use]
    pub fn with_context(mut self, new_context: impl Into<tera::Context>) -> Self {
        let context = self.context.get_or_insert_with(tera::Context::default);
        context.extend(new_context.into());

        self
    }

    /// Adds a key-value pair to the context of the Prompt
    #[must_use]
    pub fn with_context_value(mut self, key: &str, value: impl Into<tera::Value>) -> Self {
        let context = self.context.get_or_insert_with(tera::Context::default);
        context.insert(key, &value.into());
        self
    }

    /// Renders a prompt
    ///
    /// If no context is provided, the prompt will be rendered as is.
    ///
    /// # Errors
    ///
    /// See `Template::render`
    ///
    /// # Panics
    ///
    /// Panics if the `RWLock` is poisoned.
    pub fn render(&self) -> Result<String> {
        if self.context.is_none()
            && let TemplateRef::OneOff(ref template) = self.template_ref
        {
            return Ok(template.to_string());
        }

        let context: Cow<'_, tera::Context> = self
            .context
            .as_ref()
            .map_or_else(|| Cow::Owned(tera::Context::default()), Cow::Borrowed);

        match &self.template_ref {
            TemplateRef::OneOff(template) => {
                tera::Tera::one_off(template.as_ref(), &context, false)
                    .context("Failed to render one-off template")
            }
            TemplateRef::Tera(template) => TERA
                .read()
                .unwrap()
                .render(template.as_ref(), &context)
                .context("Failed to render template"),
        }
    }
}

impl From<&'static str> for Prompt {
    fn from(prompt: &'static str) -> Self {
        Prompt {
            template_ref: TemplateRef::OneOff(prompt.into()),
            context: None,
        }
    }
}

impl From<String> for Prompt {
    fn from(prompt: String) -> Self {
        Prompt {
            template_ref: TemplateRef::OneOff(prompt.into()),
            context: None,
        }
    }
}
