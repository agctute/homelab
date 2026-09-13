use super::visual::VisualLibraryApp;
use crate::config::Config;

pub fn app() -> VisualLibraryApp {
    VisualLibraryApp::new(
        "manga",
        "Comics/graphic novels whose country of origin IS Asian — Japanese manga, Korean \
         manhwa, Chinese manhua, etc. Determine origin from the author/creator and the \
         work's name, not reading direction. If the work's country of origin is not Asian, \
         it belongs to the `comics` app instead.",
        |c: &Config| c.manga_path.as_str(),
    )
}
