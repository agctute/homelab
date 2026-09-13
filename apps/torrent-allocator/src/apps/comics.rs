use super::visual::VisualLibraryApp;
use crate::config::Config;

pub fn app() -> VisualLibraryApp {
    VisualLibraryApp::new(
        "comics",
        "Comics and graphic novels whose country of origin is NOT Asian (e.g. American, \
         European, and other non-Asian creators/publishers). Determine origin from the \
         author/creator and the work's name. If the work originates from an Asian country \
         (Japan, Korea, China, etc. — manga, manhwa, manhua), it belongs to the `manga` app \
         instead, regardless of reading direction.",
        |c: &Config| c.comics_path.as_str(),
    )
}
