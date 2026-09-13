use super::prose::ProseLibraryApp;
use crate::config::Config;
use crate::format::convert::TargetFormat;

pub fn app() -> ProseLibraryApp {
    ProseLibraryApp::new(
        "novels",
        "Prose novels and light novels that are NOT textbooks — each one becomes a single \
         EPUB file in its own directory.",
        |c: &Config| c.novels_path.as_str(),
        TargetFormat::Epub,
    )
}
