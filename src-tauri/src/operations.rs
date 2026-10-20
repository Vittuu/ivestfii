//! Prevent installation from racing with a new write or chat turn.
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    active: usize,
    installing: bool,
}
#[derive(Clone, Default)]
pub struct OperationGate(Arc<Mutex<State>>);
pub struct OperationPermit(OperationGate);
pub struct InstallPermit(OperationGate);

impl OperationGate {
    pub fn begin(&self) -> Result<OperationPermit, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "Controle de operações indisponível.")?;
        if state.installing {
            return Err(
                "O aplicativo está instalando uma atualização. Aguarde a reabertura.".into(),
            );
        }
        state.active += 1;
        Ok(OperationPermit(self.clone()))
    }
    pub fn try_install(&self) -> Option<InstallPermit> {
        let mut state = self.0.lock().ok()?;
        if state.installing || state.active != 0 {
            return None;
        }
        state.installing = true;
        Some(InstallPermit(self.clone()))
    }
}
impl Drop for OperationPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0 .0.lock() {
            state.active = state.active.saturating_sub(1);
        }
    }
}
impl Drop for InstallPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0 .0.lock() {
            state.installing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_block_install_and_install_blocks_new_writes_until_guard_releases() {
        let gate = OperationGate::default();
        let write = gate.begin().unwrap();
        assert!(gate.try_install().is_none());
        drop(write);
        let install = gate.try_install().unwrap();
        assert!(gate.begin().is_err());
        assert!(gate.try_install().is_none());
        drop(install);
        assert!(gate.begin().is_ok());
    }
    #[test]
    fn overlapping_writes_all_have_to_finish() {
        let gate = OperationGate::default();
        let first = gate.begin().unwrap();
        let second = gate.begin().unwrap();
        drop(first);
        assert!(gate.try_install().is_none());
        drop(second);
        assert!(gate.try_install().is_some());
    }
}
