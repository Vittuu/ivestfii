use crate::{ConnectorCapabilities, ConnectorSpecification, ProviderStatus};
use std::{
    env,
    path::{Path, PathBuf},
};

fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        // Prefer direct executables and official command wrappers. Merely detect
        // them here; shell invocation and authentication are not implemented.
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{name}.{ext}"))
            .collect()
    } else {
        vec![name.to_owned()]
    }
}

fn find_in_directories(names: &[&str], directories: &[PathBuf]) -> Option<PathBuf> {
    // A broken shell shim must not hide a valid native installation later in
    // PATH or in the official per-user install directory.
    if cfg!(windows) {
        for directory in directories {
            for name in names {
                let path = directory.join(format!("{name}.exe"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    for directory in directories {
        for name in names {
            for candidate in executable_names(name) {
                let path = directory.join(candidate);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn directories_from_environment() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    if let Some(profile) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        let profile = Path::new(&profile);
        directories.extend([
            profile.join(".local/bin"),
            profile.join(".cargo/bin"),
            profile.join("AppData/Roaming/npm"),
            profile.join("AppData/Local/agy/bin"),
            profile.join("AppData/Local/Programs/Antigravity/bin"),
        ]);
    }
    for (key, suffix) in [("APPDATA", "npm"), ("LOCALAPPDATA", "agy/bin")] {
        if let Some(root) = env::var_os(key) {
            directories.push(Path::new(&root).join(suffix));
        }
    }
    #[cfg(windows)]
    if let Some(root) = env::var_os("APPDATA") {
        let npm = PathBuf::from(root).join("npm/node_modules/@openai");
        let (package, target) = if cfg!(target_arch = "aarch64") {
            ("codex-win32-arm64", "aarch64-pc-windows-msvc")
        } else {
            ("codex-win32-x64", "x86_64-pc-windows-msvc")
        };
        directories.push(npm.join(package).join("vendor").join(target).join("bin"));
        directories.push(
            npm.join("codex/node_modules/@openai")
                .join(package)
                .join("vendor")
                .join(target)
                .join("bin"),
        );
        directories.push(npm.join("codex/vendor").join(target).join("bin"));
    }
    directories
}

fn detect_in(directories: &[PathBuf]) -> Vec<ProviderStatus> {
    [
        ("codex", "Codex", vec!["codex"], "app-server"),
        ("claude", "Claude", vec!["claude"], "stream-json"),
        (
            "antigravity",
            "Antigravity",
            vec!["agy", "antigravity"],
            "stream-json",
        ),
    ]
    .into_iter()
    .map(|(id, name, aliases, transport)| {
        let executable = find_in_directories(&aliases, directories);
        ProviderStatus {
            id: id.into(),
            name: name.into(),
            installed: executable.is_some(),
            executable: executable.map(|path| path.as_os_str().to_string_lossy().into_owned()),
            transport: transport.into(),
        }
    })
    .collect()
}

/// Presence is not readiness: the runtime probes versions separately and never
/// reads or exports the CLI's login credentials.
pub fn detect_providers() -> Vec<ProviderStatus> {
    detect_in(&directories_from_environment())
}

/// Adapter contracts. Installed versions and authentication are checked by the
/// runtime; capabilities here describe implemented code, not an account login.
pub fn connector_specifications() -> Vec<ConnectorSpecification> {
    [
        ("codex", "app-server", vec!["app-server"]),
        (
            "claude",
            "stream-json",
            vec!["-p", "--output-format", "stream-json", "--verbose"],
        ),
        (
            "antigravity",
            "stream-json",
            vec![
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
            ],
        ),
    ]
    .into_iter()
    .map(
        |(provider_id, transport, arguments)| ConnectorSpecification {
            provider_id: provider_id.into(),
            transport: transport.into(),
            proposed_arguments: arguments.into_iter().map(str::to_owned).collect(),
            capabilities: ConnectorCapabilities {
                streaming: true,
                model_selection: true,
                mcp_tools: true,
                canonical_context_replay: true,
                implementation_status: "implemented".into(),
            },
            verification_required: vec![
                "Manter a CLI em versão compatível com o protocolo do adaptador.".into(),
                "Autenticar pelo fluxo oficial da CLI, sem transferir credenciais ao aplicativo."
                    .into(),
                "Autorizar as ferramentas de leitura do MCP pelo mecanismo do provedor.".into(),
            ],
        },
    )
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_present_providers_without_running_them() {
        let temporary = tempfile::tempdir().unwrap();
        let name = executable_names("agy").remove(0);
        std::fs::write(
            temporary.path().join(name),
            "This file must never be executed",
        )
        .unwrap();
        let result = detect_in(&[temporary.path().to_path_buf()]);
        assert!(!result[0].installed);
        assert!(!result[1].installed);
        assert!(result[2].installed);
        assert_eq!(result[2].id, "antigravity");
    }

    #[test]
    fn implemented_adapters_describe_protocol_without_claiming_account_login() {
        for connector in connector_specifications() {
            assert_eq!(connector.capabilities.implementation_status, "implemented");
            assert!(connector.capabilities.mcp_tools);
            assert!(connector.capabilities.canonical_context_replay);
        }
    }

    #[cfg(windows)]
    #[test]
    fn native_cli_takes_precedence_over_a_broken_shell_shim() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("claude.cmd"), "broken shim").unwrap();
        std::fs::write(second.path().join("claude.exe"), "presence only").unwrap();
        let result = detect_in(&[first.path().into(), second.path().into()]);
        assert_eq!(
            result[1].executable.as_deref(),
            second.path().join("claude.exe").to_str()
        );
    }
}
