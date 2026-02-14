//! In-process paste buffer store for tmux CC compatibility.
//!
//! Models tmux's named paste buffer stack. Auto-named buffers (`buffer0`,
//! `buffer1`, ...) are capped at `BUFFER_LIMIT`; user-named buffers are
//! unlimited. Buffers are ordered by insertion time (most recent first).

/// Maximum number of auto-named buffers before the oldest is evicted.
const BUFFER_LIMIT: usize = 50;

/// A single paste buffer entry.
#[derive(Debug, Clone)]
pub struct PasteBuffer {
    pub name: String,
    pub data: String,
    pub automatic: bool,
    /// Monotonic insertion order — lower = newer.
    pub order: u64,
}

/// Ordered collection of paste buffers, keyed by name.
#[derive(Debug, Clone, Default)]
pub struct PasteBufferStore {
    buffers: Vec<PasteBuffer>,
    next_order: u64,
    next_auto_index: u64,
}

impl PasteBufferStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a buffer. If `name` is `None`, auto-assign a name.
    /// Returns the buffer name used.
    pub fn set(&mut self, name: Option<&str>, data: String) -> String {
        let (buf_name, automatic) = match name {
            Some(n) => (n.to_string(), false),
            None => {
                let n = format!("buffer{}", self.next_auto_index);
                self.next_auto_index += 1;
                (n, true)
            }
        };

        // Remove existing buffer with the same name.
        self.buffers.retain(|b| b.name != buf_name);

        let order = self.next_order;
        self.next_order += 1;

        self.buffers.push(PasteBuffer {
            name: buf_name.clone(),
            data,
            automatic,
            order,
        });

        // Enforce limit on automatic buffers.
        self.enforce_limit();

        buf_name
    }

    /// Append data to an existing buffer. Returns `Err` if the buffer doesn't
    /// exist.
    pub fn append(&mut self, name: &str, data: &str) -> Result<(), String> {
        match self.buffers.iter_mut().find(|b| b.name == name) {
            Some(buf) => {
                buf.data.push_str(data);
                Ok(())
            }
            None => Err(format!("unknown buffer: {}", name)),
        }
    }

    /// Get buffer content by name. Returns `None` if not found.
    pub fn get(&self, name: &str) -> Option<&PasteBuffer> {
        self.buffers.iter().find(|b| b.name == name)
    }

    /// Get the most recently inserted buffer.
    pub fn most_recent(&self) -> Option<&PasteBuffer> {
        self.buffers.iter().max_by_key(|b| b.order)
    }

    /// Delete a buffer by name. Returns `true` if it existed.
    pub fn delete(&mut self, name: &str) -> bool {
        let len_before = self.buffers.len();
        self.buffers.retain(|b| b.name != name);
        self.buffers.len() < len_before
    }

    /// Delete the most recently inserted automatic buffer.
    /// Returns the name of the deleted buffer, or `None` if no buffers exist.
    pub fn delete_most_recent(&mut self) -> Option<String> {
        let name = self.most_recent()?.name.clone();
        self.delete(&name);
        Some(name)
    }

    /// List all buffers ordered by insertion time (newest first).
    pub fn list(&self) -> Vec<&PasteBuffer> {
        let mut sorted: Vec<&PasteBuffer> = self.buffers.iter().collect();
        sorted.sort_by(|a, b| b.order.cmp(&a.order));
        sorted
    }

    /// Number of buffers.
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    /// Evict oldest automatic buffers when limit is exceeded.
    fn enforce_limit(&mut self) {
        let auto_count = self.buffers.iter().filter(|b| b.automatic).count();
        if auto_count <= BUFFER_LIMIT {
            return;
        }
        // Find oldest automatic buffers to evict.
        let mut auto_bufs: Vec<(usize, u64)> = self
            .buffers
            .iter()
            .enumerate()
            .filter(|(_, b)| b.automatic)
            .map(|(i, b)| (i, b.order))
            .collect();
        auto_bufs.sort_by_key(|&(_, order)| order);
        let to_remove = auto_count - BUFFER_LIMIT;
        let remove_indices: Vec<usize> = auto_bufs.iter().take(to_remove).map(|&(i, _)| i).collect();
        // Remove in reverse index order to preserve indices.
        for &idx in remove_indices.iter().rev() {
            self.buffers.remove(idx);
        }
    }
}

/// Generate a `#{buffer_sample}` preview: first 50 chars, with control
/// characters escaped as octal, truncated with "..." if longer.
pub fn buffer_sample(data: &str) -> String {
    let max_len = 50;
    let mut sample = String::with_capacity(max_len + 4);
    let mut char_count = 0;
    for ch in data.chars() {
        if char_count >= max_len {
            sample.push_str("...");
            break;
        }
        if ch == '\n' {
            sample.push_str("\\n");
        } else if ch == '\r' {
            sample.push_str("\\r");
        } else if ch == '\t' {
            sample.push_str("\\t");
        } else if ch.is_control() {
            sample.push_str(&format!("\\{:03o}", ch as u32));
        } else {
            sample.push(ch);
        }
        char_count += 1;
    }
    sample
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_auto_named() {
        let mut store = PasteBufferStore::new();
        let name = store.set(None, "hello".to_string());
        assert_eq!(name, "buffer0");
        assert_eq!(store.get("buffer0").unwrap().data, "hello");
    }

    #[test]
    fn set_user_named() {
        let mut store = PasteBufferStore::new();
        let name = store.set(Some("mybuf"), "data".to_string());
        assert_eq!(name, "mybuf");
        assert!(!store.get("mybuf").unwrap().automatic);
    }

    #[test]
    fn set_replaces_existing() {
        let mut store = PasteBufferStore::new();
        store.set(Some("buf"), "old".to_string());
        store.set(Some("buf"), "new".to_string());
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("buf").unwrap().data, "new");
    }

    #[test]
    fn auto_naming_increments() {
        let mut store = PasteBufferStore::new();
        assert_eq!(store.set(None, "a".to_string()), "buffer0");
        assert_eq!(store.set(None, "b".to_string()), "buffer1");
        assert_eq!(store.set(None, "c".to_string()), "buffer2");
    }

    #[test]
    fn most_recent() {
        let mut store = PasteBufferStore::new();
        store.set(Some("first"), "1".to_string());
        store.set(Some("second"), "2".to_string());
        assert_eq!(store.most_recent().unwrap().name, "second");
    }

    #[test]
    fn delete() {
        let mut store = PasteBufferStore::new();
        store.set(Some("buf"), "data".to_string());
        assert!(store.delete("buf"));
        assert!(store.get("buf").is_none());
        assert!(!store.delete("nonexistent"));
    }

    #[test]
    fn delete_most_recent() {
        let mut store = PasteBufferStore::new();
        store.set(None, "a".to_string());
        store.set(None, "b".to_string());
        let deleted = store.delete_most_recent();
        assert_eq!(deleted, Some("buffer1".to_string()));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn list_ordered_newest_first() {
        let mut store = PasteBufferStore::new();
        store.set(Some("old"), "1".to_string());
        store.set(Some("new"), "2".to_string());
        let names: Vec<&str> = store.list().iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["new", "old"]);
    }

    #[test]
    fn append_existing() {
        let mut store = PasteBufferStore::new();
        store.set(Some("buf"), "hello".to_string());
        store.append("buf", " world").unwrap();
        assert_eq!(store.get("buf").unwrap().data, "hello world");
    }

    #[test]
    fn append_nonexistent() {
        let mut store = PasteBufferStore::new();
        assert!(store.append("nope", "data").is_err());
    }

    #[test]
    fn enforce_limit() {
        let mut store = PasteBufferStore::new();
        for _ in 0..55 {
            store.set(None, "x".to_string());
        }
        let auto_count = store.buffers.iter().filter(|b| b.automatic).count();
        assert_eq!(auto_count, 50);
    }

    #[test]
    fn user_named_not_evicted() {
        let mut store = PasteBufferStore::new();
        store.set(Some("keep_me"), "important".to_string());
        for _ in 0..55 {
            store.set(None, "x".to_string());
        }
        assert!(store.get("keep_me").is_some());
    }

    #[test]
    fn buffer_sample_short() {
        assert_eq!(buffer_sample("hello"), "hello");
    }

    #[test]
    fn buffer_sample_with_escapes() {
        assert_eq!(buffer_sample("line1\nline2\r\n"), "line1\\nline2\\r\\n");
    }

    #[test]
    fn buffer_sample_truncated() {
        let long = "a".repeat(100);
        let sample = buffer_sample(&long);
        assert!(sample.ends_with("..."));
        assert!(sample.len() < 60);
    }

    #[test]
    fn empty_store() {
        let store = PasteBufferStore::new();
        assert!(store.is_empty());
        assert!(store.most_recent().is_none());
    }
}
