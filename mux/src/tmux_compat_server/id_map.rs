//! Bidirectional ID mapping between WezTerm and tmux ID spaces.
//!
//! WezTerm uses `usize` IDs for panes and tabs, and `String` workspace names.
//! Tmux uses prefixed IDs: `%N` (panes), `@N` (windows), `$N` (sessions).
//! This module provides O(1) bidirectional lookups between the two.

use std::collections::{HashMap, HashSet};

/// WezTerm's PaneId type
pub type PaneId = usize;
/// WezTerm's TabId type
pub type TabId = usize;
/// WezTerm's mux WindowId type (distinct from tmux window IDs)
pub type MuxWindowId = usize;

/// Bidirectional mapping between WezTerm IDs and tmux IDs.
pub struct IdMap {
    // pane mappings
    wez_to_tmux_pane: HashMap<PaneId, u64>,
    tmux_to_wez_pane: HashMap<u64, PaneId>,
    next_pane_id: u64,

    // tab (window) mappings
    wez_to_tmux_window: HashMap<TabId, u64>,
    tmux_to_wez_window: HashMap<u64, TabId>,
    next_window_id: u64,

    // workspace (session) mappings
    workspace_to_tmux_session: HashMap<String, u64>,
    tmux_to_workspace: HashMap<u64, String>,
    next_session_id: u64,

    // mux window tracking (for %window-close and %sessions-changed)
    mux_window_tabs: HashMap<MuxWindowId, HashSet<TabId>>,
    mux_window_workspace: HashMap<MuxWindowId, String>,
}

impl IdMap {
    pub fn new() -> Self {
        IdMap {
            wez_to_tmux_pane: HashMap::new(),
            tmux_to_wez_pane: HashMap::new(),
            next_pane_id: 0,

            wez_to_tmux_window: HashMap::new(),
            tmux_to_wez_window: HashMap::new(),
            next_window_id: 0,

            workspace_to_tmux_session: HashMap::new(),
            tmux_to_workspace: HashMap::new(),
            next_session_id: 0,

            mux_window_tabs: HashMap::new(),
            mux_window_workspace: HashMap::new(),
        }
    }

    // --- Pane ID mappings ---

    /// Get or create a tmux pane ID for a WezTerm pane.
    pub fn get_or_create_tmux_pane_id(&mut self, wez_id: PaneId) -> u64 {
        if let Some(&tmux_id) = self.wez_to_tmux_pane.get(&wez_id) {
            return tmux_id;
        }
        let tmux_id = self.next_pane_id;
        self.next_pane_id += 1;
        self.wez_to_tmux_pane.insert(wez_id, tmux_id);
        self.tmux_to_wez_pane.insert(tmux_id, wez_id);
        tmux_id
    }

    /// Look up a WezTerm pane ID from a tmux pane ID.
    pub fn wezterm_pane_id(&self, tmux_id: u64) -> Option<PaneId> {
        self.tmux_to_wez_pane.get(&tmux_id).copied()
    }

    /// Look up a tmux pane ID from a WezTerm pane ID.
    pub fn tmux_pane_id(&self, wez_id: PaneId) -> Option<u64> {
        self.wez_to_tmux_pane.get(&wez_id).copied()
    }

    /// Remove a pane mapping by WezTerm pane ID.
    pub fn remove_pane(&mut self, wez_id: PaneId) {
        if let Some(tmux_id) = self.wez_to_tmux_pane.remove(&wez_id) {
            self.tmux_to_wez_pane.remove(&tmux_id);
        }
    }

    // --- Tab/Window ID mappings ---

    /// Get or create a tmux window ID for a WezTerm tab.
    pub fn get_or_create_tmux_window_id(&mut self, wez_id: TabId) -> u64 {
        if let Some(&tmux_id) = self.wez_to_tmux_window.get(&wez_id) {
            return tmux_id;
        }
        let tmux_id = self.next_window_id;
        self.next_window_id += 1;
        self.wez_to_tmux_window.insert(wez_id, tmux_id);
        self.tmux_to_wez_window.insert(tmux_id, wez_id);
        tmux_id
    }

    /// Look up a WezTerm tab ID from a tmux window ID.
    pub fn wezterm_tab_id(&self, tmux_id: u64) -> Option<TabId> {
        self.tmux_to_wez_window.get(&tmux_id).copied()
    }

    /// Look up a tmux window ID from a WezTerm tab ID.
    pub fn tmux_window_id(&self, wez_id: TabId) -> Option<u64> {
        self.wez_to_tmux_window.get(&wez_id).copied()
    }

    /// Remove a tab/window mapping by WezTerm tab ID.
    pub fn remove_window(&mut self, wez_id: TabId) {
        if let Some(tmux_id) = self.wez_to_tmux_window.remove(&wez_id) {
            self.tmux_to_wez_window.remove(&tmux_id);
        }
    }

    // --- Workspace/Session mappings ---

    /// Get or create a tmux session ID for a WezTerm workspace.
    pub fn get_or_create_tmux_session_id(&mut self, workspace: &str) -> u64 {
        if let Some(&tmux_id) = self.workspace_to_tmux_session.get(workspace) {
            return tmux_id;
        }
        let tmux_id = self.next_session_id;
        self.next_session_id += 1;
        self.workspace_to_tmux_session
            .insert(workspace.to_string(), tmux_id);
        self.tmux_to_workspace
            .insert(tmux_id, workspace.to_string());
        tmux_id
    }

    /// Look up a workspace name from a tmux session ID.
    pub fn workspace_name(&self, tmux_id: u64) -> Option<&str> {
        self.tmux_to_workspace.get(&tmux_id).map(|s| s.as_str())
    }

    /// Look up a tmux session ID from a workspace name.
    pub fn tmux_session_id(&self, workspace: &str) -> Option<u64> {
        self.workspace_to_tmux_session.get(workspace).copied()
    }

    /// Remove a session mapping by workspace name.
    pub fn remove_session(&mut self, workspace: &str) {
        if let Some(tmux_id) = self.workspace_to_tmux_session.remove(workspace) {
            self.tmux_to_workspace.remove(&tmux_id);
        }
    }

    /// Rename a session: re-key the workspace mapping, preserving the tmux session ID.
    /// Returns the tmux session ID if the old workspace was known, or `None`.
    pub fn rename_session(&mut self, old_workspace: &str, new_workspace: &str) -> Option<u64> {
        let tmux_id = self.workspace_to_tmux_session.remove(old_workspace)?;
        self.workspace_to_tmux_session
            .insert(new_workspace.to_string(), tmux_id);
        self.tmux_to_workspace
            .insert(tmux_id, new_workspace.to_string());
        // Update mux_window_workspace entries that referenced the old name
        for ws in self.mux_window_workspace.values_mut() {
            if ws == old_workspace {
                *ws = new_workspace.to_string();
            }
        }
        Some(tmux_id)
    }

    // --- Mux window tracking (for %window-close and %sessions-changed) ---

    /// Record that a tab belongs to a mux window in a given workspace.
    pub fn track_tab_in_window(
        &mut self,
        mux_window_id: MuxWindowId,
        tab_id: TabId,
        workspace: &str,
    ) {
        self.mux_window_tabs
            .entry(mux_window_id)
            .or_default()
            .insert(tab_id);
        self.mux_window_workspace
            .entry(mux_window_id)
            .or_insert_with(|| workspace.to_string());
    }

    /// Record a mux window's workspace (called on WindowCreated).
    pub fn track_mux_window_workspace(&mut self, mux_window_id: MuxWindowId, workspace: &str) {
        self.mux_window_workspace
            .insert(mux_window_id, workspace.to_string());
    }

    /// Get the workspace name for a mux window.
    pub fn mux_window_workspace(&self, mux_window_id: MuxWindowId) -> Option<&str> {
        self.mux_window_workspace
            .get(&mux_window_id)
            .map(|s| s.as_str())
    }

    /// Get the set of tab IDs tracked for a mux window.
    pub fn tabs_in_mux_window(&self, mux_window_id: MuxWindowId) -> Option<&HashSet<TabId>> {
        self.mux_window_tabs.get(&mux_window_id)
    }

    /// Remove all tracking for a mux window, returning the tab IDs that were in it.
    pub fn remove_mux_window(&mut self, mux_window_id: MuxWindowId) -> HashSet<TabId> {
        self.mux_window_workspace.remove(&mux_window_id);
        self.mux_window_tabs
            .remove(&mux_window_id)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pane_create_and_lookup() {
        let mut map = IdMap::new();
        assert_eq!(map.get_or_create_tmux_pane_id(42), 0);
        assert_eq!(map.get_or_create_tmux_pane_id(42), 0); // idempotent
        assert_eq!(map.get_or_create_tmux_pane_id(99), 1); // next ID
    }

    #[test]
    fn test_pane_reverse_lookup() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_pane_id(42);
        assert_eq!(map.wezterm_pane_id(0), Some(42));
        assert_eq!(map.wezterm_pane_id(999), None);
    }

    #[test]
    fn test_pane_forward_lookup() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_pane_id(42);
        assert_eq!(map.tmux_pane_id(42), Some(0));
        assert_eq!(map.tmux_pane_id(100), None);
    }

    #[test]
    fn test_pane_remove() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_pane_id(42);
        map.remove_pane(42);
        assert_eq!(map.tmux_pane_id(42), None);
        assert_eq!(map.wezterm_pane_id(0), None);
    }

    #[test]
    fn test_pane_remove_nonexistent() {
        let mut map = IdMap::new();
        map.remove_pane(999); // should not panic
    }

    #[test]
    fn test_window_create_and_lookup() {
        let mut map = IdMap::new();
        assert_eq!(map.get_or_create_tmux_window_id(10), 0);
        assert_eq!(map.get_or_create_tmux_window_id(10), 0);
        assert_eq!(map.get_or_create_tmux_window_id(20), 1);
    }

    #[test]
    fn test_window_reverse_lookup() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_window_id(10);
        assert_eq!(map.wezterm_tab_id(0), Some(10));
        assert_eq!(map.wezterm_tab_id(5), None);
    }

    #[test]
    fn test_window_remove() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_window_id(10);
        map.remove_window(10);
        assert_eq!(map.tmux_window_id(10), None);
        assert_eq!(map.wezterm_tab_id(0), None);
    }

    #[test]
    fn test_session_create_and_lookup() {
        let mut map = IdMap::new();
        assert_eq!(map.get_or_create_tmux_session_id("default"), 0);
        assert_eq!(map.get_or_create_tmux_session_id("default"), 0);
        assert_eq!(map.get_or_create_tmux_session_id("work"), 1);
    }

    #[test]
    fn test_session_workspace_name() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_session_id("default");
        assert_eq!(map.workspace_name(0), Some("default"));
        assert_eq!(map.workspace_name(5), None);
    }

    #[test]
    fn test_session_forward_lookup() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_session_id("default");
        assert_eq!(map.tmux_session_id("default"), Some(0));
        assert_eq!(map.tmux_session_id("nonexistent"), None);
    }

    #[test]
    fn test_session_remove() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_session_id("default");
        map.remove_session("default");
        assert_eq!(map.tmux_session_id("default"), None);
        assert_eq!(map.workspace_name(0), None);
    }

    #[test]
    fn test_independent_id_spaces() {
        let mut map = IdMap::new();
        // Pane, window, and session IDs are independent
        assert_eq!(map.get_or_create_tmux_pane_id(1), 0);
        assert_eq!(map.get_or_create_tmux_window_id(1), 0);
        assert_eq!(map.get_or_create_tmux_session_id("s"), 0);
        // Each has its own counter
        assert_eq!(map.get_or_create_tmux_pane_id(2), 1);
        assert_eq!(map.get_or_create_tmux_window_id(2), 1);
        assert_eq!(map.get_or_create_tmux_session_id("t"), 1);
    }

    #[test]
    fn test_many_panes() {
        let mut map = IdMap::new();
        for i in 0..100 {
            assert_eq!(map.get_or_create_tmux_pane_id(i), i as u64);
        }
        for i in 0..100 {
            assert_eq!(map.wezterm_pane_id(i as u64), Some(i));
            assert_eq!(map.tmux_pane_id(i), Some(i as u64));
        }
    }

    #[test]
    fn test_rename_session() {
        let mut map = IdMap::new();
        let sid = map.get_or_create_tmux_session_id("old");
        assert_eq!(sid, 0);
        let result = map.rename_session("old", "new");
        assert_eq!(result, Some(0));
        // Old name gone, new name points to same ID
        assert_eq!(map.tmux_session_id("old"), None);
        assert_eq!(map.tmux_session_id("new"), Some(0));
        assert_eq!(map.workspace_name(0), Some("new"));
    }

    #[test]
    fn test_rename_session_unknown() {
        let mut map = IdMap::new();
        assert_eq!(map.rename_session("nonexistent", "new"), None);
    }

    #[test]
    fn test_track_tab_in_window() {
        let mut map = IdMap::new();
        map.track_tab_in_window(1, 10, "default");
        map.track_tab_in_window(1, 20, "default");
        let tabs = map.tabs_in_mux_window(1).unwrap();
        assert!(tabs.contains(&10));
        assert!(tabs.contains(&20));
        assert_eq!(tabs.len(), 2);
        assert_eq!(map.mux_window_workspace(1), Some("default"));
    }

    #[test]
    fn test_remove_mux_window() {
        let mut map = IdMap::new();
        map.track_tab_in_window(1, 10, "default");
        map.track_tab_in_window(1, 20, "default");
        let removed = map.remove_mux_window(1);
        assert!(removed.contains(&10));
        assert!(removed.contains(&20));
        assert!(map.tabs_in_mux_window(1).is_none());
        assert!(map.mux_window_workspace(1).is_none());
    }

    #[test]
    fn test_remove_mux_window_unknown() {
        let mut map = IdMap::new();
        let removed = map.remove_mux_window(999);
        assert!(removed.is_empty());
    }

    #[test]
    fn test_rename_session_updates_mux_window_workspace() {
        let mut map = IdMap::new();
        map.get_or_create_tmux_session_id("old");
        map.track_mux_window_workspace(1, "old");
        map.track_mux_window_workspace(2, "old");
        map.rename_session("old", "new");
        assert_eq!(map.mux_window_workspace(1), Some("new"));
        assert_eq!(map.mux_window_workspace(2), Some("new"));
    }
}
