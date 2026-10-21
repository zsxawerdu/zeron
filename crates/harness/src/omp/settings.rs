//! OMP settings descriptors, generated from the pinned upstream source
//! by `scripts/gen-omp-settings.py` (see that file for provenance and
//! regeneration). This is the durable schema contract for the settings
//! service (handoff W06): types, enum values, defaults, env names,
//! credential flags, and panel metadata — never scraped from
//! `omp config list` output, whose descriptors are incomplete.
//!
//! Regenerate on every OMP pin bump and update the pin assertions in
//! the tests below.

use std::sync::OnceLock;

use serde::Deserialize;

/// Upstream revision the vendored descriptors were generated from; must
/// match the W00 pin (asserted in tests).
pub const DESCRIPTOR_REVISION: &str = "d5cd750e41fede277f9eb933455da92a0738915e";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmpSettingOption {
    pub value: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "opt_string")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    pub description: Option<String>,
}

/// UI copy fields: upstream sometimes computes them (template strings),
/// which the generator emits as `{"$expr": …}`; those read as absent
/// rather than failing the whole catalog.
fn opt_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(serde_json::Value::deserialize(deserializer)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned)))
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmpSettingUi {
    #[serde(default, deserialize_with = "opt_string")]
    pub tab: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    pub group: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "opt_string")]
    pub description: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default, deserialize_with = "opt_string")]
    pub condition: Option<String>,
    /// Submenu options, `"runtime"` (populated at runtime), or absent.
    pub options: Option<serde_json::Value>,
}

impl OmpSettingUi {
    /// Options when statically declared; empty for runtime-populated or
    /// free-form controls.
    pub fn static_options(&self) -> Vec<OmpSettingOption> {
        match &self.options {
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// One settings descriptor as declared upstream. `default` keeps its
/// JSON value; computed (non-literal) upstream defaults arrive as
/// `{"$expr": "<source>"}` and must be presented as such, never as a
/// fabricated value.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmpSettingDescriptor {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
    pub default: Option<serde_json::Value>,
    pub env: Option<String>,
    #[serde(default)]
    pub credential: bool,
    pub ui: Option<OmpSettingUi>,
}

impl OmpSettingDescriptor {
    /// String enum values (non-string declaration values are skipped).
    pub fn enum_values(&self) -> Vec<&str> {
        self.values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect()
    }

    /// Secret for display purposes: explicit credential marker or the
    /// upstream `ui.secret` flag. Secrets are masked in UI and never
    /// logged.
    pub fn is_secret(&self) -> bool {
        self.credential || self.ui.as_ref().is_some_and(|ui| ui.secret)
    }
}

#[derive(Deserialize)]
struct DescriptorsFile {
    #[allow(dead_code)]
    revision: String,
    settings: Vec<OmpSettingDescriptor>,
}

/// All descriptors from the vendored generated file, parsed once.
pub fn descriptors() -> &'static [OmpSettingDescriptor] {
    static DESCRIPTORS: OnceLock<Vec<OmpSettingDescriptor>> = OnceLock::new();
    DESCRIPTORS.get_or_init(|| {
        let file: DescriptorsFile =
            serde_json::from_str(include_str!("settings_descriptors.json"))
                .expect("vendored OMP settings descriptors parse");
        debug_assert_eq!(file.revision, DESCRIPTOR_REVISION);
        file.settings
    })
}

// ── Settings service (OMP-owned reads/writes via `omp config`) ─────────────

/// One effective value from `omp config list --json`. Credentials are
/// redacted by OMP itself (value omitted); `redacted` mirrors that.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmpEffectiveSetting {
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub redacted: bool,
    #[serde(rename = "type")]
    pub kind: String,
}

/// Outcome of a validated global write (`omp config set`): the value
/// saved to the global layer, plus whatever still shadows it (env var,
/// project layer, overlay, runtime) when OMP reports one.
#[derive(Debug, Clone, PartialEq)]
pub struct OmpSettingWrite {
    pub saved: serde_json::Value,
    pub shadowed_by: Option<String>,
}

/// Reads/writes OMP settings through the supported admin CLI. The CLI
/// validates against the same registry the descriptors came from, so
/// Zeron never edits `config.yml` (or any credential store) directly.
/// Writes land in the GLOBAL layer of the selected profile; project
/// scope is read-effective only here (project YAML editing is a
/// separate, later concern). Credential settings can be read through
/// the redacted list but never set through this service — secret entry
/// belongs to OMP's own auth flows, and argv is visible in `ps`.
pub struct OmpSettingsService<'a> {
    executable: &'a super::OmpExecutable,
    profile: Option<String>,
}

impl<'a> OmpSettingsService<'a> {
    pub fn new(executable: &'a super::OmpExecutable, profile: Option<String>) -> Self {
        Self {
            executable,
            profile,
        }
    }

    fn run_config(
        &self,
        cwd: &std::path::Path,
        args: &[&str],
    ) -> Result<serde_json::Value, crate::HarnessError> {
        let mut command = std::process::Command::new(&self.executable.path);
        if let Some(profile) = &self.profile {
            command.args(["--profile", profile]);
        }
        command
            .arg("config")
            .args(args)
            .arg("--json")
            .current_dir(cwd)
            .stdin(std::process::Stdio::null());
        crate::compose_std_child_path(&mut command, &self.executable.path);
        let output = command.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::HarnessError::Protocol(format!(
                "omp config {} failed: {}",
                args.first().copied().unwrap_or_default(),
                crate::redact_secrets(stderr.trim())
            )));
        }
        serde_json::from_slice(&output.stdout).map_err(|error| {
            crate::HarnessError::Protocol(format!("omp config output unparseable: {error}"))
        })
    }

    /// Every setting's effective value for `cwd` (project layer of that
    /// directory included), credentials redacted by OMP.
    pub fn list_effective(
        &self,
        cwd: &std::path::Path,
    ) -> Result<std::collections::BTreeMap<String, OmpEffectiveSetting>, crate::HarnessError>
    {
        let value = self.run_config(cwd, &["list"])?;
        serde_json::from_value(value).map_err(|error| {
            crate::HarnessError::Protocol(format!("omp config list shape changed: {error}"))
        })
    }

    /// Validate + write one setting to the global layer. Unknown keys
    /// and credential settings are refused before spawning anything.
    pub fn set_global(
        &self,
        cwd: &std::path::Path,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<OmpSettingWrite, crate::HarnessError> {
        let descriptor = require_writable(key)?;
        let _ = descriptor;
        let text = render_value_text(value);
        let out = self.run_config(cwd, &["set", key, &text])?;
        Ok(OmpSettingWrite {
            saved: out.get("value").cloned().unwrap_or(serde_json::Value::Null),
            shadowed_by: out
                .get("overriddenBy")
                .or_else(|| out.get("fallbackEnv"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    }

    /// Remove the global override (`omp config reset` = unset, NOT
    /// write-the-default: later OMP default changes still apply).
    pub fn unset_global(
        &self,
        cwd: &std::path::Path,
        key: &str,
    ) -> Result<(), crate::HarnessError> {
        require_writable(key)?;
        self.run_config(cwd, &["reset", key])?;
        Ok(())
    }
}

fn require_writable(key: &str) -> Result<&'static OmpSettingDescriptor, crate::HarnessError> {
    let descriptor = descriptor(key).ok_or_else(|| {
        crate::HarnessError::Protocol(format!("unknown OMP setting: {key}"))
    })?;
    if descriptor.is_secret() {
        return Err(crate::HarnessError::Protocol(format!(
            "refusing to set credential setting {key} through argv; use OMP's auth flows"
        )));
    }
    Ok(descriptor)
}

/// Render a JSON value the way `omp config set` parses it: bare scalars,
/// JSON for arrays/records.
fn render_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Bool(flag) => flag.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

/// One descriptor by dotted id (`tools.approvalMode`).
pub fn descriptor(id: &str) -> Option<&'static OmpSettingDescriptor> {
    descriptors().iter().find(|descriptor| descriptor.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_cover_the_pinned_catalog() {
        let all = descriptors();
        assert!(all.len() >= 550, "expected the full catalog, got {}", all.len());
        let approval = descriptor("tools.approvalMode").expect("approval mode descriptor");
        assert_eq!(approval.kind.as_deref(), Some("enum"));
        assert_eq!(
            approval.enum_values(),
            ["always-ask", "always-ask-cached", "write", "yolo"]
        );
        assert_eq!(
            approval.default,
            Some(serde_json::Value::String("yolo".into()))
        );
        let ui = approval.ui.as_ref().expect("panel metadata");
        assert_eq!(ui.label.as_deref(), Some("Tool Approval"));
        assert_eq!(ui.static_options().len(), 4);
        // Every id is a dotted path and unique.
        let mut ids: Vec<&str> = all.iter().map(|d| d.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
    }

    #[test]
    fn credential_settings_are_flagged_for_masking() {
        let secrets: Vec<&str> = descriptors()
            .iter()
            .filter(|d| d.is_secret())
            .map(|d| d.id.as_str())
            .collect();
        assert!(!secrets.is_empty(), "expected some credential settings");
    }
}
