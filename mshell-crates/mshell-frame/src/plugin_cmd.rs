//! Running custom-widget and plugin commands (`sh -c`).
//!
//! A plugin's `type = "secret"` settings are never part of the command text
//! (`/proc/<pid>/cmdline` is world-readable, and `exec` re-runs on every
//! poll): the bridge turns `{{key}}` into `${MPLUGIN_SECRET_<KEY>}` and lists
//! the secrets in `secret_env`. Here each one is read from the keyring off the
//! GTK thread and exported to the child (`/proc/<pid>/environ` is owner-only).

use mshell_config::schema::config::PluginSecretEnv;

/// The `(var, value)` pairs for `refs`, read from the keyring on a blocking
/// worker. A secret that isn't set is exported as an empty string, so the
/// command sees the variable defined either way.
pub(crate) async fn secret_envs(refs: &[PluginSecretEnv]) -> Vec<(String, String)> {
    if refs.is_empty() {
        return Vec::new();
    }
    let refs = refs.to_vec();
    tokio::task::spawn_blocking(move || {
        refs.into_iter()
            .map(|r| {
                let value = mshell_plugins::secrets::read(&r.plugin, &r.key).unwrap_or_default();
                (r.var, value)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// `sh -c <cmd>` with `envs` exported.
pub(crate) fn shell(cmd: &str, envs: &[(String, String)]) -> tokio::process::Command {
    let mut c = tokio::process::Command::new("sh");
    c.arg("-c").arg(cmd);
    for (var, value) in envs {
        c.env(var, value);
    }
    c
}

/// Fire-and-forget a command; the child is reaped to avoid zombies.
pub(crate) fn run_detached(cmd: &str, refs: &[PluginSecretEnv]) {
    let cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        return;
    }
    let refs = refs.to_vec();
    relm4::spawn(async move {
        let envs = secret_envs(&refs).await;
        let _ = shell(&cmd, &envs).status().await;
    });
}

/// Run a command and capture its stdout. `None` on spawn failure or a
/// non-zero exit.
pub(crate) async fn run_capture(cmd: &str, refs: &[PluginSecretEnv]) -> Option<String> {
    let envs = secret_envs(refs).await;
    let out = shell(cmd, &envs).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The secret reaches the child through its environment while the
    /// command line only names the variable.
    #[tokio::test]
    async fn secret_reaches_the_child_but_not_its_argv() {
        let cmd = "printf %s \"${MPLUGIN_SECRET_K}\"; tr '\\0' ' ' < /proc/$$/cmdline >&2";
        let envs = vec![("MPLUGIN_SECRET_K".to_string(), "sk-123".to_string())];
        let out = shell(cmd, &envs).output().await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "sk-123");
        let argv = String::from_utf8_lossy(&out.stderr);
        assert!(argv.contains("MPLUGIN_SECRET_K"), "{argv}");
        assert!(!argv.contains("sk-123"), "{argv}");
    }

    #[tokio::test]
    async fn no_refs_means_no_keyring_access() {
        assert!(secret_envs(&[]).await.is_empty());
    }
}
