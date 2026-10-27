mod model;
mod ui;
use appcore::prelude::*;
struct FontsApp {
    view: ui::FontsView,
}
impl App for FontsApp {
    type Body = ui::FontsView;
    fn new() -> Self {
        let app = Self {
            view: ui::FontsView::new(),
        };
        if let Some(path) = std::env::args_os()
            .skip(1)
            .map(std::path::PathBuf::from)
            .find(|path| {
                path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
                    ["ttf", "otf", "ttc", "otc"]
                        .iter()
                        .any(|ext| s.eq_ignore_ascii_case(ext))
                })
            })
        {
            app.view.open_path(path);
        }
        app
    }
    fn window(&self) -> WindowOptions {
        WindowOptions::new("Fonts")
            .size(1000.0, 700.0)
            .resizable(true)
    }
    fn body(&self, _: &ViewContext) -> Self::Body {
        self.view.clone()
    }
    fn handle_platform_message_with_handles(
        &mut self,
        message: &[u8],
        handles: &mut [PlatformFileHandle],
    ) -> bool {
        match document::decode_open_message(message, handles) {
            Ok(Some(document)) => {
                self.view.open_delegated(document);
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.view.open_error(error.to_string());
                true
            }
        }
    }
}
fn main() -> Result<(), ViewKitError> {
    run::<FontsApp>()
}
