//! Setup progress contains no balances, credentials or inferred investor data.
use std::{fs, io::Write, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Progress {
    pub version: u8,
    pub wallet_id: String,
    pub step: u8,
    pub status: ProgressStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus {
    Started,
    Skipped,
    Completed,
}

impl Progress {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.step > 2
            || self.wallet_id.is_empty()
            || self.wallet_id.len() > 200
        {
            return Err(
                "Progresso dos primeiros passos inválido. O arquivo foi preservado.".into(),
            );
        }
        Ok(())
    }
}

pub fn read(path: &Path) -> Result<Option<Progress>, String> {
    match fs::read(path) {
        Ok(bytes) => {
            let progress: Progress = serde_json::from_slice(&bytes).map_err(|_| {
                "Não foi possível ler os primeiros passos. O arquivo foi preservado."
            })?;
            progress.validate()?;
            Ok(Some(progress))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn save(path: &Path, progress: &Progress) -> Result<(), String> {
    progress.validate()?;
    // Never replace an unreadable or future progress file with defaults.
    if read(path)?.as_ref() == Some(progress) {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(progress).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

/// Existing portfolios, declared profiles and conversations must not trigger a
/// welcome dialog on upgrade. Only a genuinely untouched default is eligible.
pub fn eligible(
    wallet_count: usize,
    customized: bool,
    has_investments: bool,
    has_profile: bool,
    has_conversations: bool,
) -> bool {
    wallet_count == 1 && !customized && !has_investments && !has_profile && !has_conversations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_data_or_user_choices_never_trigger_an_upgrade_welcome() {
        assert!(eligible(1, false, false, false, false));
        for input in [
            (2, false, false, false, false),
            (1, true, false, false, false),
            (1, false, true, false, false),
            (1, false, false, true, false),
            (1, false, false, false, true),
        ] {
            assert!(!eligible(input.0, input.1, input.2, input.3, input.4));
        }
    }

    #[test]
    fn skip_resume_and_completion_survive_restart_without_financial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        assert_eq!(read(&path).unwrap(), None);
        for (step, status) in [
            (1, ProgressStatus::Started),
            (1, ProgressStatus::Skipped),
            (2, ProgressStatus::Completed),
        ] {
            let progress = Progress {
                version: 1,
                wallet_id: "principal".into(),
                step,
                status,
            };
            save(&path, &progress).unwrap();
            let bytes = fs::read(&path).unwrap();
            save(&path, &progress).unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(read(&path).unwrap(), Some(progress));
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_future_and_invalid_progress_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        let valid = Progress {
            version: 1,
            wallet_id: "principal".into(),
            step: 0,
            status: ProgressStatus::Skipped,
        };
        for bytes in [
            b"broken".to_vec(),
            br#"{"version":2,"walletId":"principal","step":0,"status":"started"}"#.to_vec(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(save(&path, &valid).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        let mut invalid = valid;
        invalid.step = 3;
        assert!(invalid.validate().is_err());
    }
}
