mod model;
mod ui;
use appcore::prelude::*;
struct FontsApp {
    view: ui::FontsView,
}
impl App for FontsApp {
    type Body = ui::FontsView;
    fn new() -> Self {
        Self {
            view: ui::FontsView::new(),
        }
    }
    fn window(&self) -> WindowOptions {
        WindowOptions::new("Fonts")
            .size(1000.0, 700.0)
            .resizable(true)
    }
    fn body(&self, _: &ViewContext) -> Self::Body {
        self.view.clone()
    }
}
fn main() -> Result<(), ViewKitError> {
    run::<FontsApp>()
}
