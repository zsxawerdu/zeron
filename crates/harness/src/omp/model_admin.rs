//! Model/role administration (W05): persistent role assignments,
//! cycle order, fallback chains and the default thinking level, read
//! and written through OMP's supported admin routes (`omp config`,
//! `omp usage`) — never by editing `config.yml` or any credential
//! store directly. This is the persistent layer; a session's current
//! model is a separate, session-only selection (`set_model`).
//!
//! Role names are OMP's vocabulary: the built-ins below, plus any
//! custom role a profile already uses. Custom roles are discovered
//! from config and preserved, never pruned to the built-in list.

use std::collections::BTreeMap;
use std::path::Path;

use zeron_proto::omp::{OmpModelRoles, OmpProviderLimits};

use super::settings::{OmpEffectiveSetting, OmpSettingsService};
use super::OmpExecutable;
use crate::HarnessError;

/// Roles OMP ships with (role shortcuts in the reference hub).
/// Profiles may add custom roles; those are merged in from config.
pub const BUILTIN_ROLES: &[&str] = &[
    "default", "smol", "slow", "vision", "plan", "commit", "tiny", "memory", "task", "advisor",
    "widget", "web", "judge",
];

/// Parse the persistent role snapshot out of an effective-settings
/// map (`omp config list`). Non-string record entries are skipped:
/// OMP's `modelRoles` is a string record, and a value we cannot
/// represent must not be silently rewritten on the next save —
/// writes below re-read the raw record before mutating it.
pub fn parse_model_roles(effective: &BTreeMap<String, OmpEffectiveSetting>) -> OmpModelRoles {
    let string_record = |key: &str| -> BTreeMap<String, String> {
        effective
            .get(key)
            .and_then(|setting| setting.value.as_ref())
            .and_then(serde_json::Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(name, value)| {
                        value.as_str().map(|text| (name.clone(), text.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let assignments = string_record("modelRoles");
    let cycle_order: Vec<String> = effective
        .get("cycleOrder")
        .and_then(|setting| setting.value.as_ref())
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let fallback_chains: BTreeMap<String, Vec<String>> = effective
        .get("retry.fallbackChains")
        .and_then(|setting| setting.value.as_ref())
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_array().map(|items| {
                        (
                            key.clone(),
                            items
                                .iter()
                                .filter_map(|item| item.as_str().map(str::to_owned))
                                .collect(),
                        )
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut roles: Vec<String> = BUILTIN_ROLES.iter().map(ToString::to_string).collect();
    for role in assignments.keys().chain(cycle_order.iter()) {
        if !roles.contains(role) {
            roles.push(role.clone());
        }
    }
    OmpModelRoles {
        roles,
        assignments,
        cycle_order,
        fallback_chains,
        default_thinking_level: effective
            .get("defaultThinkingLevel")
            .and_then(|setting| setting.value.as_ref())
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        storage: effective
            .get("modelRoleStorage")
            .and_then(|setting| setting.value.as_ref())
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    }
}

fn require_role(role: &str) -> Result<(), HarnessError> {
    if role.is_empty() || role.chars().any(char::is_whitespace) {
        return Err(HarnessError::Protocol(format!(
            "invalid OMP role name: {role:?}"
        )));
    }
    Ok(())
}

fn require_selector(selector: &str) -> Result<(), HarnessError> {
    if selector.trim().is_empty() {
        return Err(HarnessError::Protocol("empty model selector".into()));
    }
    Ok(())
}

/// Synchronous admin operations behind [`super::OmpHarness`]'s async
/// wrappers. Holds only the executable + profile; every operation
/// shells to OMP's own CLI, which validates and persists.
pub struct OmpModelAdmin {
    executable: OmpExecutable,
    profile: Option<String>,
}

impl OmpModelAdmin {
    pub fn new(executable: OmpExecutable, profile: Option<String>) -> Self {
        Self {
            executable,
            profile,
        }
    }

    fn settings(&self) -> OmpSettingsService<'_> {
        OmpSettingsService::new(&self.executable, self.profile.clone())
    }

    pub fn model_roles(&self, cwd: &Path) -> Result<OmpModelRoles, HarnessError> {
        let effective = self.settings().list_effective(cwd)?;
        Ok(parse_model_roles(&effective))
    }

    /// Assign `selector` to `role` persistently, or clear the role's
    /// explicit assignment with `None` (OMP's automatic resolution
    /// then applies). The raw record is re-read and mutated as JSON
    /// so entries this build does not model survive the write.
    pub fn set_model_role(
        &self,
        cwd: &Path,
        role: &str,
        selector: Option<&str>,
    ) -> Result<OmpModelRoles, HarnessError> {
        require_role(role)?;
        if let Some(selector) = selector {
            require_selector(selector)?;
        }
        let service = self.settings();
        let effective = service.list_effective(cwd)?;
        let mut record: serde_json::Map<String, serde_json::Value> = effective
            .get("modelRoles")
            .and_then(|setting| setting.value.as_ref())
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        match selector {
            Some(selector) => {
                record.insert(role.to_owned(), serde_json::Value::String(selector.to_owned()));
            }
            None => {
                record.remove(role);
            }
        }
        service.set_global(cwd, "modelRoles", &serde_json::Value::Object(record))?;
        self.model_roles(cwd)
    }

    /// Replace the persistent cycle order (role names, hub order).
    pub fn set_cycle_order(
        &self,
        cwd: &Path,
        order: &[String],
    ) -> Result<OmpModelRoles, HarnessError> {
        for role in order {
            require_role(role)?;
        }
        self.settings().set_global(
            cwd,
            "cycleOrder",
            &serde_json::Value::Array(
                order
                    .iter()
                    .map(|role| serde_json::Value::String(role.clone()))
                    .collect(),
            ),
        )?;
        self.model_roles(cwd)
    }

    /// Replace one fallback chain (`key`: a role, a provider/model
    /// selector, or a `provider/*` wildcard). An empty chain removes
    /// the entry. Raw re-read + mutate, as with role assignments.
    pub fn set_fallback_chain(
        &self,
        cwd: &Path,
        key: &str,
        chain: &[String],
    ) -> Result<OmpModelRoles, HarnessError> {
        if key.trim().is_empty() {
            return Err(HarnessError::Protocol("empty fallback chain key".into()));
        }
        for selector in chain {
            require_selector(selector)?;
        }
        let service = self.settings();
        let effective = service.list_effective(cwd)?;
        let mut record: serde_json::Map<String, serde_json::Value> = effective
            .get("retry.fallbackChains")
            .and_then(|setting| setting.value.as_ref())
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        if chain.is_empty() {
            record.remove(key);
        } else {
            record.insert(
                key.to_owned(),
                serde_json::Value::Array(
                    chain
                        .iter()
                        .map(|selector| serde_json::Value::String(selector.clone()))
                        .collect(),
                ),
            );
        }
        service.set_global(cwd, "retry.fallbackChains", &serde_json::Value::Object(record))?;
        self.model_roles(cwd)
    }

    /// Set the persistent default thinking level (an opaque token;
    /// OMP validates it against the setting's enum).
    pub fn set_default_thinking(
        &self,
        cwd: &Path,
        level: &str,
    ) -> Result<OmpModelRoles, HarnessError> {
        if level.trim().is_empty() {
            return Err(HarnessError::Protocol("empty thinking level".into()));
        }
        self.settings().set_global(
            cwd,
            "defaultThinkingLevel",
            &serde_json::Value::String(level.to_owned()),
        )?;
        self.model_roles(cwd)
    }

    /// Provider usage limits via `omp usage --json`: OMP fetches and
    /// accounts them; report internals stay raw native records.
    pub fn provider_limits(&self) -> Result<OmpProviderLimits, HarnessError> {
        let mut command = std::process::Command::new(&self.executable.path);
        if let Some(profile) = &self.profile {
            command.args(["--profile", profile]);
        }
        command
            .args(["usage", "--json"])
            .stdin(std::process::Stdio::null());
        crate::compose_std_child_path(&mut command, &self.executable.path);
        let output = command.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(HarnessError::Protocol(format!(
                "omp usage failed: {}",
                crate::redact_secrets(stderr.trim())
            )));
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            HarnessError::Protocol(format!("omp usage output unparseable: {error}"))
        })?;
        let array = |key: &str| -> Vec<serde_json::Value> {
            value
                .get(key)
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        Ok(OmpProviderLimits {
            generated_at_ms: value
                .get("generatedAt")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default(),
            reports: array("reports"),
            accounts_without_usage: array("accountsWithoutUsage"),
            disabled_credentials: array("disabledCredentials"),
            capacity: value.get("capacity").cloned().unwrap_or(serde_json::Value::Null),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn effective(pairs: Vec<(&str, serde_json::Value)>) -> BTreeMap<String, OmpEffectiveSetting> {
        pairs
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_owned(),
                    OmpEffectiveSetting {
                        value: Some(value),
                        redacted: false,
                        kind: "record".into(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn parse_roles_keeps_custom_roles_and_chains() {
        let roles = parse_model_roles(&effective(vec![
            (
                "modelRoles",
                json!({"default": "anthropic/claude-opus-5", "reviewer": "@slow"}),
            ),
            ("cycleOrder", json!(["smol", "default", "reviewer"])),
            (
                "retry.fallbackChains",
                json!({"default": ["openai/gpt-5.2"], "google/*": ["deepseek/deepseek-v4"]}),
            ),
            ("defaultThinkingLevel", json!("xhigh")),
            ("modelRoleStorage", json!("global")),
        ]));
        assert_eq!(roles.assignments["default"], "anthropic/claude-opus-5");
        // Automatic resolution through another role is kept verbatim.
        assert_eq!(roles.assignments["reviewer"], "@slow");
        // The custom role is offered after the built-ins, not dropped.
        assert_eq!(roles.roles.last().map(String::as_str), Some("reviewer"));
        assert_eq!(roles.cycle_order, ["smol", "default", "reviewer"]);
        assert_eq!(roles.fallback_chains["google/*"], ["deepseek/deepseek-v4"]);
        assert_eq!(roles.default_thinking_level.as_deref(), Some("xhigh"));
        assert_eq!(roles.storage.as_deref(), Some("global"));
    }

    #[test]
    fn parse_roles_tolerates_absent_config() {
        let roles = parse_model_roles(&BTreeMap::new());
        assert_eq!(roles.roles.len(), BUILTIN_ROLES.len());
        assert!(roles.assignments.is_empty() && roles.cycle_order.is_empty());
        assert!(roles.default_thinking_level.is_none());
    }

    #[test]
    fn role_and_selector_validation_rejects_blank() {
        assert!(require_role("slow").is_ok());
        assert!(require_role("").is_err());
        assert!(require_role("two words").is_err());
        assert!(require_selector("openai/gpt-5.2").is_ok());
        assert!(require_selector("  ").is_err());
    }
}
