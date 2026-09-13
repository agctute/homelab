use super::prose::ProseLibraryApp;
use crate::config::Config;
use crate::format::convert::TargetFormat;

pub fn app() -> ProseLibraryApp {
    ProseLibraryApp::new(
        "textbooks",
        "Academic/reference textbooks — each one becomes a single PDF file in its own \
         directory.",
        |c: &Config| c.textbooks_path.as_str(),
        TargetFormat::Pdf,
    )
}
