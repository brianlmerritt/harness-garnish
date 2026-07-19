use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub task_id: Option<String>,
    pub severity: String,
}

pub trait Notifier: Send + Sync {
    fn notify(&self, notification: &Notification) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct FakeNotifier {
    delivered: Arc<Mutex<Vec<Notification>>>,
}

impl FakeNotifier {
    pub fn delivered(&self) -> Vec<Notification> {
        self.delivered.lock().expect("notification lock").clone()
    }
}

impl Notifier for FakeNotifier {
    fn notify(&self, notification: &Notification) -> Result<()> {
        self.delivered
            .lock()
            .expect("notification lock")
            .push(notification.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_notifier_is_deterministic_and_local() {
        let notifier = FakeNotifier::default();
        let notification = Notification {
            title: "Review ready".into(),
            body: "Task t1 awaits review".into(),
            task_id: Some("t1".into()),
            severity: "info".into(),
        };
        notifier.notify(&notification).unwrap();
        assert_eq!(notifier.delivered(), vec![notification]);
    }
}
