use serde::Serialize;
use std::{path::PathBuf, sync::{Arc, Mutex, Weak}};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const LIMIT: usize = 32;
type Cancel = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ReceiveState { Receiving, Saving, Stopping, Completed, Stopped }
impl ReceiveState {
    fn finished(self) -> bool { matches!(self, Self::Completed | Self::Stopped) }
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedFile {
    pub task_id: String,
    pub name: String,
    pub size: u64,
    pub received_bytes: u64,
    pub state: ReceiveState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
struct Record { info: ReceivedFile, directory: PathBuf, cancel: Cancel }
#[derive(Default)]
struct State { next: u64, records: Vec<Record> }
pub struct ReceiveHistory { state: Mutex<State>, open_slot: Arc<Semaphore> }
impl Default for ReceiveHistory {
    fn default() -> Self { Self { state: Mutex::new(State::default()), open_slot: Arc::new(Semaphore::new(1)) } }
}

impl ReceiveHistory {
    pub(crate) fn begin(self: &Arc<Self>, name: String, size: u64, directory: PathBuf,
        device_name: Option<String>, cancel: Cancel) -> Result<ReceiveObserver, String> {
        let mut state = self.state.lock().unwrap();
        if state.records.len() == LIMIT {
            let Some(index) = state.records.iter().rposition(|record| record.info.state.finished()) else {
                return Err("接收记录已满，请等待正在接收的文件完成后重试".into());
            };
            state.records.remove(index);
        }
        state.next += 1;
        let task_id = format!("{:032x}", state.next);
        state.records.insert(0, Record { info: ReceivedFile { task_id: task_id.clone(), name, size,
            received_bytes: 0, state: ReceiveState::Receiving, device_name, error: None }, directory, cancel });
        Ok(ReceiveObserver { history: Arc::downgrade(self), task_id })
    }
    pub fn list(&self) -> Vec<ReceivedFile> {
        let mut result: Vec<_> = self.state.lock().unwrap().records.iter().map(|record| record.info.clone()).collect();
        result.sort_by_key(|record| record.state.finished());
        result
    }
    pub fn cancel(&self, task_id: &str) -> Result<(), String> {
        let cancel = {
            let mut state = self.state.lock().unwrap();
            let record = state.records.iter_mut().find(|record| record.info.task_id == task_id)
                .ok_or("接收记录已不存在，请刷新后重试")?;
            if record.info.state.finished() { return Ok(()); }
            record.info.state = ReceiveState::Stopping;
            record.cancel.clone()
        };
        cancel();
        Ok(())
    }
    pub fn clear_finished(&self) { self.state.lock().unwrap().records.retain(|record| !record.info.state.finished()); }
    pub fn directory(&self, task_id: &str) -> Result<PathBuf, String> {
        let state = self.state.lock().unwrap();
        let record = state.records.iter().find(|record| record.info.task_id == task_id)
            .ok_or("接收记录已不存在，请刷新后重试")?;
        if record.info.state != ReceiveState::Completed { return Err("文件尚未保存完成，暂时无法打开目录".into()); }
        Ok(record.directory.clone())
    }
    pub fn try_open_slot(&self) -> Result<OwnedSemaphorePermit, String> {
        self.open_slot.clone().try_acquire_owned().map_err(|_| "正在打开接收目录，请稍候".into())
    }
}

pub(crate) struct ReceiveObserver { history: Weak<ReceiveHistory>, task_id: String }
impl ReceiveObserver {
    fn update(&self, change: impl FnOnce(&mut ReceivedFile)) {
        let Some(history) = self.history.upgrade() else { return; };
        let mut state = history.state.lock().unwrap();
        if let Some(record) = state.records.iter_mut().find(|record| record.info.task_id == self.task_id) {
            if !record.info.state.finished() { change(&mut record.info); }
        }
    }
    pub(crate) fn received(&self, bytes: u64) {
        self.update(|info| { info.received_bytes = bytes.min(info.size); info.error = None;
            if info.state != ReceiveState::Stopping { info.state = ReceiveState::Receiving; } });
    }
    pub(crate) fn saving(&self) {
        self.update(|info| { info.error = None; if info.state != ReceiveState::Stopping { info.state = ReceiveState::Saving; } });
    }
    pub(crate) fn stopping(&self) { self.update(|info| { info.state = ReceiveState::Stopping; }); }
    pub(crate) fn completed(&self, name: &str) {
        self.update(|info| { info.name = name.into(); info.received_bytes = info.size; info.state = ReceiveState::Completed; info.error = None; });
    }
    pub(crate) fn error(&self, message: &str) {
        self.update(|info| { info.error = Some(message.chars().take(512).collect());
            if info.state != ReceiveState::Stopping { info.state = ReceiveState::Receiving; } });
    }
    pub(crate) fn stopped(&self) { self.update(|info| { info.state = ReceiveState::Stopped; }); }
}
impl Drop for ReceiveObserver { fn drop(&mut self) { self.stopped(); } }

#[cfg(test)]
mod tests {
    use super::*;
    fn begin(history: &Arc<ReceiveHistory>) -> ReceiveObserver {
        history.begin("file.bin".into(), 10, PathBuf::from("original"), Some("phone".into()), Arc::new(|| {})).unwrap()
    }
    #[test]
    fn progress_saving_and_actual_completion_are_distinct() {
        let history = Arc::new(ReceiveHistory::default()); let observer = begin(&history);
        observer.received(10); assert_eq!(history.list()[0].state, ReceiveState::Receiving);
        observer.saving(); assert_eq!(history.list()[0].state, ReceiveState::Saving);
        observer.completed("file (1).bin"); drop(observer);
        let record = &history.list()[0]; assert_eq!(record.name, "file (1).bin"); assert_eq!(record.state, ReceiveState::Completed);
        assert_eq!(history.directory(&record.task_id).unwrap(), PathBuf::from("original"));
        let json = serde_json::to_string(record).unwrap(); assert!(!json.contains("original")); assert!(!json.contains("cancel"));
    }
    #[test]
    fn cancellation_runs_after_unlock_and_completion_wins() {
        let history = Arc::new(ReceiveHistory::default()); let weak = Arc::downgrade(&history);
        let observer = history.begin("file".into(), 0, PathBuf::new(), None, Arc::new(move || {
            assert_eq!(weak.upgrade().unwrap().list()[0].state, ReceiveState::Stopping);
        })).unwrap();
        let id = history.list()[0].task_id.clone(); history.cancel(&id).unwrap();
        assert!(history.directory(&id).is_err()); observer.completed("file"); history.cancel(&id).unwrap();
        assert_eq!(history.list()[0].state, ReceiveState::Completed);
    }
    #[test]
    fn clear_and_late_observers_do_not_resurrect_results_or_remove_active_tasks() {
        let history = Arc::new(ReceiveHistory::default()); let complete = begin(&history); complete.completed("file");
        let active = begin(&history); history.clear_finished(); complete.received(2); complete.completed("late");
        assert_eq!(history.list().len(), 1); assert_eq!(history.list()[0].state, ReceiveState::Receiving);
        drop(active); assert_eq!(history.list()[0].state, ReceiveState::Stopped); history.clear_finished(); assert!(history.list().is_empty());
    }
    #[test]
    fn capacity_never_evicts_active_tasks_and_only_retires_old_results() {
        let history = Arc::new(ReceiveHistory::default()); let observers: Vec<_> = (0..LIMIT).map(|_| begin(&history)).collect();
        assert!(history.begin("extra".into(), 0, PathBuf::new(), None, Arc::new(|| {})).is_err());
        observers[0].completed("oldest"); let extra = begin(&history); assert_eq!(history.list().len(), LIMIT);
        assert!(!history.list().iter().any(|record| record.name == "oldest")); drop(extra);
        assert_eq!(history.list().last().unwrap().state, ReceiveState::Stopped);
    }
    #[test]
    fn directory_open_slots_remain_held_by_the_actual_worker() {
        let history = ReceiveHistory::default(); let slot = history.try_open_slot().unwrap();
        assert!(history.try_open_slot().is_err());
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || { let _slot = slot; receive.recv().unwrap(); });
        assert!(history.try_open_slot().is_err()); send.send(()).unwrap(); worker.join().unwrap();
        assert!(history.try_open_slot().is_ok());
    }
}
