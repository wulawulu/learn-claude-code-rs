use super::*;

impl SharedTeammateManager {
    pub(super) fn request_teammate_stop(&self, name: &str) -> Result<()> {
        if let Some(stop_signal) = self.lock_stop_signals()?.get(name).cloned() {
            stop_signal.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(super) fn cleanup_finished_teammate(&self, name: &str) -> Result<()> {
        let mut handles = self.lock_handles()?;
        let handle = handles.remove(name);
        drop(handles);
        if let Some(h) = handle {
            drop(h)
        };

        let stop_signal = self.lock_stop_signals()?.remove(name);

        if let Some(s) = stop_signal {
            drop(s)
        };

        self.set_status_if_changed(name, TeammateStatus::Shutdown)
    }
}
