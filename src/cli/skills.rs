use std::{
    fs,
    path::{Path, PathBuf},
};

const SKILL_DIR: &str = ".skills/litellm-schedule";
const SKILL_FILE: &str = "SKILL.md";

const LITELLM_SCHEDULE_SKILL: &str = r#"---
name: litellm-schedule
description: Collect details for scheduling a remote LiteLLM agent.
---

# LiteLLM Scheduled Remote Agent

Use this skill when the user wants to deploy, schedule, or create a scheduled
remote LiteLLM agent.

Start by asking for exactly these details:

1. What should the agent do? Describe the task. It runs remotely with zero
   context, so the task must be self-contained.
2. Which repo? Default: https://github.com/LiteLLM-Labs/lite-harness.
3. When / how often? Recurring or one-time? The user's timezone is
   America/Los_Angeles. Minimum recurring interval is 1 hour.
4. Connectors? Slack, Linear, and Gmail are available.

Use `claude-sonnet-4-6` as the default model.

After collecting the answers, summarize the proposed schedule and stop. Do not
make an API request yet. The agents endpoint is not defined.
"#;

pub(crate) fn ensure_litellm_schedule_skill(
    root: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let skill_path = root.join(SKILL_DIR).join(SKILL_FILE);
    if skill_path.exists() {
        return Ok(skill_path);
    }

    if let Some(parent) = skill_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&skill_path, LITELLM_SCHEDULE_SKILL)?;
    Ok(skill_path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::ensure_litellm_schedule_skill;

    #[test]
    fn writes_litellm_schedule_skill_when_missing() {
        let temp_dir = TempDir::new().unwrap();

        let skill_path = ensure_litellm_schedule_skill(temp_dir.path()).unwrap();

        let skill = fs::read_to_string(skill_path).unwrap();
        assert!(skill.contains("name: litellm-schedule"));
        assert!(skill.contains("What should the agent do?"));
        assert!(skill.contains("https://github.com/LiteLLM-Labs/lite-harness"));
    }

    #[test]
    fn leaves_existing_litellm_schedule_skill_unchanged() {
        let temp_dir = TempDir::new().unwrap();
        let skill_path = temp_dir
            .path()
            .join(".skills")
            .join("litellm-schedule")
            .join("SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, "custom").unwrap();

        ensure_litellm_schedule_skill(temp_dir.path()).unwrap();

        assert_eq!(fs::read_to_string(skill_path).unwrap(), "custom");
    }
}
