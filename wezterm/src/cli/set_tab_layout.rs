use clap::Parser;
use codec::SetTabLayout;
use mux::pane::PaneId;
use wezterm_client::client::Client;

#[derive(Debug, Parser, Clone)]
pub struct CliSetTabLayout {
    /// Specify the target pane.
    /// The default is to use the current pane based on the
    /// environment variable WEZTERM_PANE.
    #[arg(long)]
    pane_id: Option<PaneId>,
    /// The layout to apply. One of: even-horizontal, even-vertical,
    /// main-horizontal, main-vertical, tiled.
    layout: String,
}

impl CliSetTabLayout {
    pub async fn run(&self, client: Client) -> anyhow::Result<()> {
        let pane_id = client.resolve_pane_id(self.pane_id).await?;
        match client
            .set_tab_layout(SetTabLayout {
                pane_id,
                layout_name: self.layout.clone(),
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(err) => Err(err),
        }
    }
}
