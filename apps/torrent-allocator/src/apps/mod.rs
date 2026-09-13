mod prose;
mod visual;

pub mod audiobooks;
pub mod comics;
pub mod manga;
pub mod novels;
pub mod textbooks;

use crate::classifier::Classification;
use crate::config::Config;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CompletedTorrent {
    pub hash: String,
    pub name: String,
    pub category: Option<String>,
    pub content_path: PathBuf,
    pub save_path: String,
}

/// One integration per downstream app. The classifier picks an app by `id()`;
/// `handle()` then owns everything specific to that app — where its files
/// belong and what should happen to them on the way there.
#[async_trait]
pub trait AppHandler: Send + Sync {
    /// Stable identifier returned by the classifier, e.g. "comics".
    fn id(&self) -> &'static str;

    /// Sent to the LLM as part of the classification prompt so it knows
    /// what belongs to this app.
    fn description(&self) -> &'static str;

    /// Copy/convert a completed torrent into this app's library. Never
    /// deletes anything under the torrent's own content_path — qBittorrent
    /// keeps seeding from there, so the source must survive untouched.
    async fn handle(
        &self,
        torrent: &CompletedTorrent,
        classification: &Classification,
        config: &Config,
    ) -> Result<()>;
}

/// Every app integration is registered here. To add a new app: implement
/// `AppHandler` in its own module under `src/apps/`, then push it onto this
/// list — nothing else in the service needs to change.
pub fn registry() -> Vec<Box<dyn AppHandler>> {
    vec![
        Box::new(comics::app()),
        Box::new(manga::app()),
        Box::new(novels::app()),
        Box::new(textbooks::app()),
        Box::new(audiobooks::app()),
    ]
}
