use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct BatchControl {
    pub paused: AtomicBool,
}

impl BatchControl {
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    pub async fn wait_until_resumed(&self) {
        while self.paused.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
}

#[tauri::command]
pub async fn resume_batch(state: tauri::State<'_, BatchControl>) -> Result<(), String> {
    state.resume();
    Ok(())
}
