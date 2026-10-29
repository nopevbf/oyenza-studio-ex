//! A small personal task list.

use serde::{Deserialize, Serialize};

pub const MAX_TASK_LEN: usize = 200;
pub const MAX_TASKS: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub text: String,
    pub done: bool,
    pub created_unix: i64,
    #[serde(default)]
    pub done_unix: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskList {
    pub tasks: Vec<Task>,
    pub next_id: u64,
}

impl Default for TaskList {
    fn default() -> Self {
        Self { tasks: Vec::new(), next_id: 1 }
    }
}

impl TaskList {
    /// Add a task; returns its id, or `None` for blank text / a full list.
    pub fn add(&mut self, text: &str, now_unix: i64) -> Option<u64> {
        let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || self.tasks.len() >= MAX_TASKS {
            return None;
        }
        let text: String = text.chars().take(MAX_TASK_LEN).collect();
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.tasks.push(Task { id, text, done: false, created_unix: now_unix, done_unix: None });
        Some(id)
    }

    /// Edit a task's text; returns true on success, false if id not found or text empty.
    pub fn edit(&mut self, id: u64, new_text: &str) -> bool {
        let text: String = new_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return false;
        }
        let text: String = text.chars().take(MAX_TASK_LEN).collect();
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.text = text;
            true
        } else {
            false
        }
    }

    pub fn toggle(&mut self, id: u64, now_unix: i64) -> bool {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.done = !t.done;
            t.done_unix = if t.done { Some(now_unix) } else { None };
            true
        } else {
            false
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        self.tasks.len() != before
    }

    pub fn clear_done(&mut self) -> usize {
        let before = self.tasks.len();
        self.tasks.retain(|t| !t.done);
        before - self.tasks.len()
    }

    pub fn open_count(&self) -> usize {
        self.tasks.iter().filter(|t| !t.done).count()
    }

    /// Tasks completed at or after `day_start_unix`.
    pub fn done_since(&self, day_start_unix: i64) -> usize {
        self.tasks.iter().filter(|t| t.done && t.done_unix.is_some_and(|d| d >= day_start_unix)).count()
    }

    /// "done today / (open + done today)" — e.g. 1/3.
    pub fn today_progress(&self, day_start_unix: i64) -> (usize, usize) {
        let done = self.done_since(day_start_unix);
        (done, done + self.open_count())
    }

    /// Whether there are pending (undone) tasks created at or after `day_start_unix`.
    pub fn has_pending_tasks_today(&self, day_start_unix: i64) -> bool {
        self.tasks.iter().any(|t| !t.done && t.created_unix >= day_start_unix)
    }

    /// Number of pending tasks created at or after `day_start_unix`.
    pub fn pending_tasks_today_count(&self, day_start_unix: i64) -> usize {
        self.tasks.iter().filter(|t| !t.done && t.created_unix >= day_start_unix).count()
    }

    /// Display order: open tasks first (oldest first), then today's done tasks.
    pub fn display_order(&self, day_start_unix: i64) -> Vec<&Task> {
        let mut open: Vec<&Task> = self.tasks.iter().filter(|t| !t.done).collect();
        let mut done: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| t.done && t.done_unix.is_some_and(|d| d >= day_start_unix))
            .collect();
        open.sort_by_key(|t| (t.created_unix, t.id));
        done.sort_by_key(|t| std::cmp::Reverse(t.done_unix));
        open.extend(done);
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_toggle_remove() {
        let mut l = TaskList::default();
        assert_eq!(l.add("   ", 0), None);
        let a = l.add("  buy   milk ", 10).unwrap();
        let b = l.add("ship poni", 20).unwrap();
        assert_eq!(l.tasks[0].text, "buy milk");
        assert!(l.toggle(a, 100));
        assert_eq!(l.today_progress(50), (1, 2));
        assert_eq!(l.today_progress(200), (0, 1));
        let order: Vec<u64> = l.display_order(50).iter().map(|t| t.id).collect();
        assert_eq!(order, vec![b, a]);
        assert_eq!(l.clear_done(), 1);
        assert!(l.remove(b));
        assert!(!l.remove(b));
    }

    #[test]
    fn truncates_long_text() {
        let mut l = TaskList::default();
        let id = l.add(&"x".repeat(500), 0).unwrap();
        assert_eq!(l.tasks.iter().find(|t| t.id == id).unwrap().text.len(), MAX_TASK_LEN);
    }

    #[test]
    fn ids_survive_default_zero() {
        let mut l = TaskList { tasks: vec![], next_id: 0 };
        assert_eq!(l.add("a", 0), Some(1));
        assert_eq!(l.add("b", 0), Some(2));
    }
    #[test]
    fn edit_task() {
        let mut l = TaskList::default();
        let a = l.add("buy milk", 10).unwrap();
        assert!(!l.edit(a, "   "));
        assert!(!l.edit(999, "new text"));
        assert!(l.edit(a, "  buy   almond   milk  "));
        assert_eq!(l.tasks[0].text, "buy almond milk");
        assert!(l.edit(a, &"x".repeat(500)));
        assert_eq!(l.tasks[0].text.len(), MAX_TASK_LEN);
    }

    #[test]
    fn pending_tasks_today() {
        let mut l = TaskList::default();
        let day_start = 1_000;
        // Task from yesterday
        let old = l.add("yesterday task", 500).unwrap();
        assert!(!l.has_pending_tasks_today(day_start));
        assert_eq!(l.pending_tasks_today_count(day_start), 0);

        // Task created today
        let today1 = l.add("today task 1", 1_200).unwrap();
        let today2 = l.add("today task 2", 1_300).unwrap();
        assert!(l.has_pending_tasks_today(day_start));
        assert_eq!(l.pending_tasks_today_count(day_start), 2);

        // Complete one task
        l.toggle(today1, 1_400);
        assert!(l.has_pending_tasks_today(day_start));
        assert_eq!(l.pending_tasks_today_count(day_start), 1);

        // Complete the other task
        l.toggle(today2, 1_500);
        assert!(!l.has_pending_tasks_today(day_start));
        assert_eq!(l.pending_tasks_today_count(day_start), 0);

        // Old task is still open, but was not created today
        assert!(!l.tasks.iter().find(|t| t.id == old).unwrap().done);
    }
}
