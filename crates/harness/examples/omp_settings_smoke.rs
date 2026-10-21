//! W06 happy-path check for the OMP settings service against an
//! ISOLATED throwaway profile (never the caller's real config):
//! list effective values, set a boolean, observe it, unset it, observe
//! the default return. Credential and unknown keys must be refused
//! before any process spawns. Deletes the profile dir afterwards.
//!   cargo run -p zeron-harness --example omp_settings_smoke
use zeron_harness::omp::settings::{descriptor, descriptors, OmpSettingsService};
use zeron_harness::omp::OmpExecutable;

const PROFILE: &str = "zeron-w06-test";


fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("descriptors vendored: {}", descriptors().len());
    let exe = OmpExecutable::resolve(None)?;
    let service = OmpSettingsService::new(&exe, Some(PROFILE.into()));
    let cwd = std::env::temp_dir().join(format!("zeron-omp-settings-{}", std::process::id()));
    std::fs::create_dir_all(&cwd)?;

    let key = "compaction.enabled";
    let default = descriptor(key).and_then(|d| d.default.clone());
    println!("descriptor default for {key}: {default:?}");

    let effective = |svc: &OmpSettingsService| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        Ok(svc
            .list_effective(&cwd)?
            .get(key)
            .and_then(|entry| entry.value.clone())
            .unwrap_or(serde_json::Value::Null))
    };

    println!("before: {:?}", effective(&service)?);
    let written = service.set_global(&cwd, key, &serde_json::json!(false))?;
    println!("set -> saved={:?} shadowed_by={:?}", written.saved, written.shadowed_by);
    let after_set = effective(&service)?;
    println!("after set: {after_set:?}");
    if after_set != serde_json::json!(false) {
        return Err("set did not take effect".into());
    }
    service.unset_global(&cwd, key)?;
    let after_unset = effective(&service)?;
    println!("after unset: {after_unset:?}");
    if Some(&after_unset) != default.as_ref() {
        return Err("unset did not restore the default".into());
    }

    // Refusals happen before any spawn.
    let credential = service.set_global(&cwd, "auth.broker.token", &serde_json::json!("x"));
    println!("credential set refused: {}", credential.is_err());
    if credential.is_ok() {
        return Err("credential setting was writable through the service".into());
    }
    if service.set_global(&cwd, "no.such.setting", &serde_json::json!(true)).is_ok() {
        return Err("unknown setting was writable".into());
    }

    // Clean up the throwaway profile and workspace.
    if let Some(home) = std::env::var_os("HOME") {
        let profile_dir = std::path::Path::new(&home)
            .join(".omp/profiles")
            .join(PROFILE);
        let _ = std::fs::remove_dir_all(profile_dir);
    }
    let _ = std::fs::remove_dir_all(&cwd);
    println!("OMP settings smoke passed");
    Ok(())
}
