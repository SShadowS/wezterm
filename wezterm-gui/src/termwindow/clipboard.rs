use crate::termwindow::TermWindowNotif;
use crate::TermWindow;
use config::keyassignment::{ClipboardCopyDestination, ClipboardPasteSource};
use mux::pane::Pane;
use mux::Mux;
use std::sync::Arc;
use window::{Clipboard, WindowOps};

impl TermWindow {
    pub fn copy_to_clipboard(&self, clipboard: ClipboardCopyDestination, text: String) {
        // Build the list of physical clipboard targets to write. On platforms
        // with a real primary selection (X11/Wayland) Clipboard and
        // PrimarySelection are distinct destinations. On Windows there is only
        // one system clipboard, so ClipboardAndPrimarySelection must not write
        // it twice: clipboard-win empties the clipboard on each open, and the
        // second open/empty/set cycle can race a clipboard-history listener and
        // leave the clipboard blank (symptom: a real entry followed by an empty
        // one, with the empty one winning on paste).
        let targets: Vec<Clipboard> = match clipboard {
            ClipboardCopyDestination::Clipboard => vec![Clipboard::Clipboard],
            ClipboardCopyDestination::PrimarySelection => vec![Clipboard::PrimarySelection],
            ClipboardCopyDestination::ClipboardAndPrimarySelection => {
                #[cfg(windows)]
                {
                    vec![Clipboard::Clipboard]
                }
                #[cfg(not(windows))]
                {
                    vec![Clipboard::Clipboard, Clipboard::PrimarySelection]
                }
            }
        };
        for c in targets {
            self.window.as_ref().unwrap().set_clipboard(c, text.clone());
        }
    }

    pub fn paste_from_clipboard(&mut self, pane: &Arc<dyn Pane>, clipboard: ClipboardPasteSource) {
        let pane_id = pane.pane_id();
        log::trace!(
            "paste_from_clipboard in pane {} {:?}",
            pane.pane_id(),
            clipboard
        );
        let window = self.window.as_ref().unwrap().clone();
        let clipboard = match clipboard {
            ClipboardPasteSource::Clipboard => Clipboard::Clipboard,
            ClipboardPasteSource::PrimarySelection => Clipboard::PrimarySelection,
        };
        let future = window.get_clipboard(clipboard);
        promise::spawn::spawn(async move {
            if let Ok(clip) = future.await {
                window.notify(TermWindowNotif::Apply(Box::new(move |myself| {
                    if let Some(pane) = myself
                        .pane_state(pane_id)
                        .overlay
                        .as_ref()
                        .map(|overlay| overlay.pane.clone())
                        .or_else(|| {
                            let mux = Mux::get();
                            mux.get_pane(pane_id)
                        })
                    {
                        pane.send_paste(&clip).ok();
                    }
                })));
            }
        })
        .detach();
        self.maybe_scroll_to_bottom_for_input(&pane);
    }
}
