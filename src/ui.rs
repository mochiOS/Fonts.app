use crate::model::{self, FaceKey, Filter, Model};
use appcore::fonts::{self, Catalogue, FontFile, Locations};
use appcore::prelude::*;
use appcore::viewkit::{
    event::{EventContext, EventResult, ViewEvent},
    view::{Constraints, MeasureContext, PaintContext},
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const ENGLISH: &str = "The quick brown fox jumps over the lazy dog.";
const JAPANESE: &str = "あーかい狐と緑のたっぬっき～";
const ROW: f32 = 48.0;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Action {
    Refresh,
    Install,
    InstallPath(PathBuf),
    Family(String),
    Face(FaceKey),
    Filter(Filter),
    English,
    Japanese,
    Back,
    Toggle,
    AskRemove,
    CancelRemove,
    ConfirmRemove,
}
struct Work {
    catalogue: Catalogue,
    notice: String,
    error: Option<String>,
    preferred: Option<FaceKey>,
}
struct Core {
    model: RefCell<Model>,
    locations: Locations,
    pending: Rc<Cell<Option<Action>>>,
    buttons: RefCell<BTreeMap<Action, ButtonInteractionState>>,
    search: TextFieldInteractionState,
    specimen: TextFieldInteractionState,
    size: State<f32>,
    filter_selection: State<usize>,
    filter_interaction: SegmentedControlInteractionState,
    size_interaction: SliderInteractionState,
    list_scroll: ScrollState,
    detail_scroll: ScrollState,
    styles_scroll: ScrollState,
    last_query: RefCell<String>,
    receiver: RefCell<Option<Receiver<Work>>>,
    removal: RefCell<Option<FontFile>>,
    panel: RefCell<Option<OpenPanel>>,
    alert: Alert,
}
impl Core {
    fn button(&self, label: impl Into<String>, action: Action) -> Button {
        let state = self
            .buttons
            .borrow_mut()
            .entry(action.clone())
            .or_default()
            .clone();
        let pending = self.pending.clone();
        Button::with_interaction_and_label(state, label)
            .size(ButtonSize::Small)
            .style(ButtonStyle::Ghost)
            .on_click(move || pending.set(Some(action.clone())))
    }
    fn start(&self, action: Action) {
        if self.receiver.borrow().is_some() {
            return;
        }
        let selected = self
            .model
            .borrow()
            .selected_face()
            .map(|(file, _)| file.clone());
        let confirmed = self.removal.borrow_mut().take();
        let locations = self.locations.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.model.borrow_mut().loading = true;
        let result = std::thread::Builder::new()
            .name("font-library".into())
            .spawn(move || {
                let mut preferred = None;
                let operation = match action {
                    Action::Refresh => Ok("Font library refreshed.".to_string()),
                    Action::InstallPath(path) => fonts::install(&locations, &path).map(|file| {
                        preferred = file.faces.first().map(|face| FaceKey::new(&file, face));
                        format!("Installed {}.", file.faces[0].family)
                    }),
                    Action::Toggle => selected
                        .as_ref()
                        .ok_or_else(|| std::io::Error::other("Select a user font first."))
                        .and_then(|file| {
                            fonts::set_enabled(&locations, file, !file.enabled).map(|_| {
                                if file.enabled {
                                    "Font disabled."
                                } else {
                                    "Font enabled."
                                }
                                .into()
                            })
                        }),
                    Action::ConfirmRemove => confirmed
                        .as_ref()
                        .ok_or_else(|| std::io::Error::other("Removal was cancelled."))
                        .and_then(|file| {
                            fonts::remove(&locations, file).map(|_| "Font file removed.".into())
                        }),
                    _ => return,
                };
                let (notice, error) = match operation {
                    Ok(message) => (message, None),
                    Err(error) => (
                        "Font library was not changed as requested.".into(),
                        Some(error.to_string()),
                    ),
                };
                let _ = sender.send(Work {
                    catalogue: fonts::catalogue(&locations),
                    notice,
                    error,
                    preferred,
                });
            });
        match result {
            Ok(_) => *self.receiver.borrow_mut() = Some(receiver),
            Err(error) => {
                let mut model = self.model.borrow_mut();
                model.loading = false;
                model.notice = format!("Could not start font operation: {error}");
            }
        }
    }
    fn poll(&self) {
        let next = self.receiver.borrow().as_ref().map(Receiver::try_recv);
        match next {
            Some(Ok(work)) => {
                self.receiver.borrow_mut().take();
                let installed = work.preferred.is_some();
                if installed {
                    self.search.set_value("");
                    self.last_query.borrow_mut().clear();
                }
                let mut model = self.model.borrow_mut();
                if installed {
                    model.filter = Filter::User;
                    self.filter_selection.set(1);
                }
                model.update(work.catalogue, &self.search.value(), work.preferred);
                if installed {
                    model.detail_open = true;
                }
                model.notice = if model.catalogue.warnings.is_empty() {
                    work.notice
                } else {
                    format!("{} Some font files were skipped.", work.notice)
                };
                if let Some(error) = work.error {
                    model.notice = error.clone();
                    self.alert.present_error("Font operation failed", error);
                }
                let names = model
                    .families
                    .iter()
                    .map(|family| family.name.as_str())
                    .collect::<std::collections::BTreeSet<_>>();
                self.buttons.borrow_mut().retain(|action, _| match action {
                    Action::Family(name) => names.contains(name.as_str()),
                    Action::Face(key) => model.resolve(key).is_some(),
                    _ => true,
                });
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.receiver.borrow_mut().take();
                let mut model = self.model.borrow_mut();
                model.loading = false;
                model.notice = "The font operation stopped. Try Refresh again.".into();
            }
            _ => {}
        }
    }
    fn apply(&self) -> bool {
        let mut changed = false;
        let query = self.search.value();
        if *self.last_query.borrow() != query {
            self.model.borrow_mut().reconcile(&query);
            *self.last_query.borrow_mut() = query.clone();
            self.list_scroll.reset();
            changed = true;
        }
        let Some(action) = self.pending.take() else {
            return changed;
        };
        if self.model.borrow().loading
            && !matches!(
                action,
                Action::Family(_)
                    | Action::Face(_)
                    | Action::Filter(_)
                    | Action::Back
                    | Action::English
                    | Action::Japanese
            )
        {
            return changed;
        }
        match action {
            Action::Refresh | Action::InstallPath(_) | Action::Toggle | Action::ConfirmRemove => {
                self.start(action)
            }
            Action::Install => {
                if let Some(panel) = self.panel.borrow().as_ref() {
                    panel.show();
                }
            }
            Action::Family(name) => {
                self.model.borrow_mut().select(&name, &query);
                self.detail_scroll.reset();
                self.styles_scroll.reset();
            }
            Action::Face(key) => {
                self.model.borrow_mut().face = Some(key);
                self.detail_scroll.reset();
            }
            Action::Filter(filter) => {
                let mut model = self.model.borrow_mut();
                model.filter = filter;
                self.filter_selection.set(match filter {
                    Filter::All => 0,
                    Filter::User => 1,
                    Filter::System => 2,
                });
                model.reconcile(&query);
                self.list_scroll.reset();
            }
            Action::English => self.specimen.set_value(ENGLISH),
            Action::Japanese => self.specimen.set_value(JAPANESE),
            Action::Back => self.model.borrow_mut().detail_open = false,
            Action::AskRemove => {
                let file = self
                    .model
                    .borrow()
                    .selected_face()
                    .map(|(file, _)| file.clone());
                *self.removal.borrow_mut() = file.filter(|file| file.managed);
            }
            Action::CancelRemove => {
                self.removal.borrow_mut().take();
            }
        }
        true
    }
}
#[derive(Clone)]
pub struct FontsView {
    core: Rc<Core>,
}
#[derive(Clone, Copy)]
struct Frames {
    toolbar: Rect,
    sidebar: Option<Rect>,
    detail: Option<Rect>,
    footer: Rect,
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w.max(0.0), h.max(0.0))
}
fn fill(bounds: Rect, color: Color, c: &mut PaintContext<'_>) {
    Rectangle::new()
        .color(RectangleColor::Custom(color))
        .paint(bounds, c);
}
impl Frames {
    fn new(bounds: Rect, detail: bool) -> Self {
        let top = bounds.size.height.min(48.0);
        let bottom = (bounds.size.height - top).clamp(0.0, 32.0);
        let body = rect(
            bounds.origin.x,
            bounds.origin.y + top,
            bounds.size.width,
            bounds.size.height - top - bottom,
        );
        let (sidebar, detail) = if bounds.size.width < 720.0 {
            ((!detail).then_some(body), detail.then_some(body))
        } else {
            (
                Some(rect(body.origin.x, body.origin.y, 260.0, body.size.height)),
                Some(rect(
                    body.origin.x + 260.0,
                    body.origin.y,
                    body.size.width - 260.0,
                    body.size.height,
                )),
            )
        };
        Self {
            toolbar: rect(bounds.origin.x, bounds.origin.y, bounds.size.width, top),
            sidebar,
            detail,
            footer: rect(
                bounds.origin.x,
                body.origin.y + body.size.height,
                bounds.size.width,
                bottom,
            ),
        }
    }
}
impl FontsView {
    pub fn new() -> Self {
        let view = Self::with_model(Model::new(), Locations::current());
        let pending = view.core.pending.clone();
        *view.core.panel.borrow_mut() = Some(OpenPanel::new(
            OpenPanelOptions {
                title: "Install Font".into(),
                allowed_content_types: ["font/ttf", "font/otf", "font/collection"]
                    .into_iter()
                    .map(|s| ContentType::parse(s).unwrap())
                    .collect(),
                ..Default::default()
            },
            move |path| {
                pending.set(Some(Action::InstallPath(path.to_owned())));
                Ok(())
            },
        ));
        view.core.start(Action::Refresh);
        view
    }
    fn with_model(model: Model, locations: Locations) -> Self {
        let specimen = TextFieldInteractionState::new();
        specimen.set_value(ENGLISH);
        Self {
            core: Rc::new(Core {
                model: RefCell::new(model),
                locations,
                pending: Rc::new(Cell::new(None)),
                buttons: RefCell::new(BTreeMap::new()),
                search: TextFieldInteractionState::new(),
                specimen,
                size: State::new(36.0),
                filter_selection: State::new(0),
                filter_interaction: SegmentedControlInteractionState::new(),
                size_interaction: SliderInteractionState::new(),
                list_scroll: ScrollState::new(),
                detail_scroll: ScrollState::new(),
                styles_scroll: ScrollState::new(),
                last_query: RefCell::new(String::new()),
                receiver: RefCell::new(None),
                removal: RefCell::new(None),
                panel: RefCell::new(None),
                alert: Alert::new(),
            }),
        }
    }
    fn toolbar(&self, bounds: Rect, compact_detail: bool) -> Vec<(Rect, Button)> {
        let busy = self.core.model.borrow().loading;
        let mut buttons = vec![
            (
                rect(
                    bounds.origin.x + bounds.size.width - 164.0,
                    bounds.origin.y + 9.0,
                    124.0,
                    30.0,
                ),
                self.core
                    .button("Install Font…", Action::Install)
                    .leading_symbol(SymbolName::Plus)
                    .style(ButtonStyle::Standard)
                    .enabled(!busy && self.core.locations.user.is_some()),
            ),
            (
                rect(
                    bounds.origin.x + bounds.size.width - 36.0,
                    bounds.origin.y + 9.0,
                    28.0,
                    30.0,
                ),
                self.core
                    .button("", Action::Refresh)
                    .leading_symbol(SymbolName::Refresh)
                    .accessibility_label("Refresh font library")
                    .enabled(!busy),
            ),
        ];
        if compact_detail {
            buttons.push((
                rect(bounds.origin.x + 8.0, bounds.origin.y + 9.0, 96.0, 30.0),
                self.core
                    .button("Fonts", Action::Back)
                    .leading_symbol(SymbolName::ChevronLeft),
            ));
        }
        buttons
    }
    fn search(&self) -> TextField {
        TextField::with_interaction(self.core.search.clone())
            .placeholder("Search fonts")
            .enabled(true)
    }
    fn filters(&self) -> SegmentedControl {
        let pending = self.core.pending.clone();
        SegmentedControl::with_interaction(
            self.core.filter_selection.binding(),
            self.core.filter_interaction.clone(),
        )
        .item(0, "All")
        .item(1, "User")
        .item(2, "System")
        .accessibility_label("Font collection")
        .on_change(move |value| {
            pending.set(Some(Action::Filter(match value {
                1 => Filter::User,
                2 => Filter::System,
                _ => Filter::All,
            })))
        })
    }
    fn sidebar(&self, b: Rect) -> Scroll {
        let viewport = rect(
            b.origin.x,
            b.origin.y + 84.0,
            b.size.width,
            b.size.height - 84.0,
        );
        Scroll::new(self.core.list_scroll.clone()).content(FamilyList {
            core: self.core.clone(),
            viewport,
        })
    }
    fn detail(&self) -> Scroll {
        Scroll::new(self.core.detail_scroll.clone()).content(Detail(self.core.clone()))
    }
    fn confirmation(&self) -> Dialog<VStack> {
        let file = self.core.removal.borrow();
        let file = file.as_ref().unwrap();
        let name = file
            .faces
            .first()
            .map(|face| face.family.as_str())
            .unwrap_or("this font");
        let short_name = name.chars().take(40).collect::<String>();
        let short_name = if name.chars().count() > 40 {
            format!("{short_name}…")
        } else {
            short_name
        };
        Dialog::new().accessibility_label("Remove font file").content(VStack::new().alignment(StackAlignment::Stretch).gap(StackGap::Medium)
            .child(Text::styled("Remove Font?",TextRole::TitleSmall))
            .child(Text::body(format!("“{short_name}” will be removed from your fonts. This file contains {} {}. System fonts are kept.",file.faces.len(),if file.faces.len()==1 {"typeface"}else{"typefaces"})))
            .child(HStack::new().alignment(StackAlignment::Center).gap(StackGap::Small)
                .child(self.core.button("Cancel",Action::CancelRemove).style(ButtonStyle::Standard))
                .child(self.core.button("Remove Font",Action::ConfirmRemove).style(ButtonStyle::Danger))))
    }
}
impl View for FontsView {
    fn measure(&self, c: Constraints, _: &mut MeasureContext<'_>) -> Size {
        c.constrain(c.maximum)
    }
    fn paint(&self, b: Rect, c: &mut PaintContext<'_>) {
        self.core.apply();
        self.core.poll();
        let model = self.core.model.borrow();
        let f = Frames::new(b, model.detail_open);
        if model.loading {
            c.request_redraw_in_at(b, Instant::now() + Duration::from_millis(80));
        }
        fill(b, c.theme.colors.background, c);
        fill(f.toolbar, c.theme.colors.surface_subtle, c);
        let compact = b.size.width < 720.0 && model.detail_open;
        if !compact {
            Text::label("Fonts").paint(
                rect(
                    b.origin.x + 16.0,
                    b.origin.y + 12.0,
                    (b.size.width - 192.0).max(0.0),
                    24.0,
                ),
                c,
            );
        }
        for (bounds, button) in self.toolbar(f.toolbar, compact) {
            button.paint(bounds, c);
        }
        if let Some(sidebar) = f.sidebar {
            fill(sidebar, c.theme.colors.surface_subtle, c);
            self.search().paint(
                rect(
                    sidebar.origin.x + 12.0,
                    sidebar.origin.y + 10.0,
                    sidebar.size.width - 24.0,
                    30.0,
                ),
                c,
            );
            self.filters().paint(
                rect(
                    sidebar.origin.x + 12.0,
                    sidebar.origin.y + 48.0,
                    sidebar.size.width - 24.0,
                    28.0,
                ),
                c,
            );
            let list = rect(
                sidebar.origin.x,
                sidebar.origin.y + 84.0,
                sidebar.size.width,
                sidebar.size.height - 84.0,
            );
            self.sidebar(sidebar).paint(list, c);
        }
        if let Some(detail) = f.detail {
            self.detail().paint(detail, c);
        }
        fill(f.footer, c.theme.colors.surface_subtle, c);
        let status = if model.loading {
            "Reading fonts…".into()
        } else if self.core.locations.user.is_none() {
            "Read-only library · User font storage is unavailable.".into()
        } else if !model.notice.is_empty() {
            model.notice.clone()
        } else if !model.catalogue.warnings.is_empty() {
            format!(
                "{} families · Some font files could not be read.",
                model.families.len()
            )
        } else {
            format!(
                "{} families · {} font files",
                model.families.len(),
                model.catalogue.files.len()
            )
        };
        Text::caption(status)
            .color(c.theme.colors.text_secondary)
            .paint(
                rect(
                    f.footer.origin.x + 12.0,
                    f.footer.origin.y + 4.0,
                    f.footer.size.width - 24.0,
                    24.0,
                ),
                c,
            );
        drop(model);
        if self.core.removal.borrow().is_some() {
            fill(b, c.theme.shell.scrim, c);
            let width = (b.size.width - 32.0).clamp(0.0, 440.0);
            self.confirmation().paint(
                rect(
                    b.origin.x + (b.size.width - width) * 0.5,
                    b.origin.y + (b.size.height - 240.0).max(0.0) * 0.5,
                    width,
                    240.0_f32.min(b.size.height),
                ),
                c,
            );
        }
        if let Some(panel) = self.core.panel.borrow().as_ref() {
            panel.paint(b, c);
        }
    }
    fn handle_event(&self, b: Rect, e: &ViewEvent, c: &mut EventContext<'_>) -> EventResult {
        if let Some(panel) = self.core.panel.borrow().as_ref() {
            if panel.is_visible() {
                let result = panel.handle_event(b, e, c);
                if self.core.apply() {
                    c.request_redraw_in(b);
                }
                return result;
            }
        }
        if self.core.removal.borrow().is_some() {
            let modal = rect(
                b.origin.x + (b.size.width - (b.size.width - 32.0).min(440.0)) * 0.5,
                b.origin.y + (b.size.height - 240.0).max(0.0) * 0.5,
                (b.size.width - 32.0).min(440.0),
                240.0_f32.min(b.size.height),
            );
            if matches!(
                e,
                ViewEvent::KeyPressed {
                    key: appcore::viewkit::platform::Key::Escape,
                    ..
                }
            ) {
                self.core.pending.set(Some(Action::CancelRemove));
            }
            let _ = self.confirmation().handle_event(modal, e, c);
            if self.core.apply() {
                c.request_redraw_in(b);
            }
            return EventResult::Consumed;
        }
        let f = Frames::new(b, self.core.model.borrow().detail_open);
        let mut result = EventResult::Ignored;
        for (bounds, button) in self.toolbar(
            f.toolbar,
            b.size.width < 720.0 && self.core.model.borrow().detail_open,
        ) {
            result = result.merge(button.handle_event(bounds, e, c));
        }
        if let Some(sidebar) = f.sidebar {
            result = result.merge(self.search().handle_event(
                rect(
                    sidebar.origin.x + 12.0,
                    sidebar.origin.y + 10.0,
                    sidebar.size.width - 24.0,
                    30.0,
                ),
                e,
                c,
            ));
            result = result.merge(self.filters().handle_event(
                rect(
                    sidebar.origin.x + 12.0,
                    sidebar.origin.y + 48.0,
                    sidebar.size.width - 24.0,
                    28.0,
                ),
                e,
                c,
            ));
            result = result.merge(self.sidebar(sidebar).handle_event(
                rect(
                    sidebar.origin.x,
                    sidebar.origin.y + 84.0,
                    sidebar.size.width,
                    sidebar.size.height - 84.0,
                ),
                e,
                c,
            ));
        }
        if let Some(detail) = f.detail {
            result = result.merge(self.detail().handle_event(detail, e, c));
        }
        if result == EventResult::Ignored
            && b.size.width < 720.0
            && matches!(
                e,
                ViewEvent::KeyPressed {
                    key: appcore::viewkit::platform::Key::Escape,
                    ..
                }
            )
        {
            self.core.pending.set(Some(Action::Back));
        }
        if self.core.apply() {
            c.request_redraw_in(b);
            EventResult::Consumed
        } else {
            result
        }
    }
}
struct FamilyList {
    core: Rc<Core>,
    viewport: Rect,
}
impl FamilyList {
    fn entries(&self) -> Vec<usize> {
        self.core.model.borrow().visible(&self.core.search.value())
    }
    fn row(&self, index: usize, theme: &Theme) -> Button {
        let model = self.core.model.borrow();
        let family = &model.families[index];
        let selected = model.selected.as_ref() == Some(&family.name);
        self.core
            .button("", Action::Family(family.name.clone()))
            .accessibility_role(AccessibilityRole::ListItem)
            .accessibility_selected(selected)
            .accessibility_label(format!("{}, {} styles", family.name, family.faces.len()))
            .style(ButtonStyle::Custom {
                background: if selected {
                    theme.list.selected_background
                } else {
                    Color::TRANSPARENT
                },
                hovered_background: theme.list.hovered_background,
                border: Color::TRANSPARENT,
                hovered_border: Color::TRANSPARENT,
                foreground: theme.list.foreground,
            })
            .alignment(ZStackAlignment::Leading)
            .content(
                VStack::new()
                    .alignment(StackAlignment::Stretch)
                    .gap(StackGap::None)
                    .child(Text::label(&family.name))
                    .child(
                        Text::caption(format!("{} styles", family.faces.len()))
                            .color(theme.colors.text_secondary),
                    ),
            )
    }
}
impl View for FamilyList {
    fn measure(&self, c: Constraints, _: &mut MeasureContext<'_>) -> Size {
        c.constrain(Size::new(
            c.maximum.width,
            self.entries().len().max(1) as f32 * ROW,
        ))
    }
    fn paint(&self, b: Rect, c: &mut PaintContext<'_>) {
        let entries = self.entries();
        if entries.is_empty() {
            Text::body(if self.core.model.borrow().loading {
                "Reading fonts…"
            } else {
                "No fonts match. Try another search or collection."
            })
            .color(c.theme.colors.text_secondary)
            .paint(
                rect(
                    b.origin.x + 16.0,
                    b.origin.y + 12.0,
                    b.size.width - 32.0,
                    100.0,
                ),
                c,
            );
        }
        for (row, index) in entries.iter().enumerate() {
            let bounds = rect(
                b.origin.x + 8.0,
                b.origin.y + row as f32 * ROW,
                b.size.width - 16.0,
                ROW - 2.0,
            );
            if bounds.intersection(self.viewport).is_some() {
                self.row(*index, c.theme).paint(bounds, c);
            }
        }
    }
    fn handle_event(&self, b: Rect, e: &ViewEvent, c: &mut EventContext<'_>) -> EventResult {
        let mut result = EventResult::Ignored;
        for (row, index) in self.entries().iter().enumerate() {
            let bounds = rect(
                b.origin.x + 8.0,
                b.origin.y + row as f32 * ROW,
                b.size.width - 16.0,
                ROW - 2.0,
            );
            if bounds.intersection(self.viewport).is_some() {
                result = result.merge(self.row(*index, c.theme()).handle_event(bounds, e, c));
            }
        }
        result
    }
}
struct Detail(Rc<Core>);
fn info(label: &str, value: impl Into<String>, theme: &Theme) -> HStack {
    HStack::new()
        .alignment(StackAlignment::Center)
        .gap(StackGap::Medium)
        .child(
            Text::caption(label)
                .color(theme.colors.text_secondary)
                .layout()
                .width(104.0)
                .flex_shrink(0.0),
        )
        .child(Text::body(value.into()).layout().flex_grow(1.0))
}
impl Detail {
    fn content(&self, theme: &Theme) -> Padding<VStack> {
        let model = self.0.model.borrow();
        let mut stack = VStack::new()
            .alignment(StackAlignment::Stretch)
            .gap(StackGap::Medium);
        let Some((file, face)) = model.selected_face() else {
            return Padding::all(theme.spacing.large).content(
                stack
                    .child(Text::styled(
                        if model.loading {
                            "Reading font library…"
                        } else {
                            "Select a font"
                        },
                        TextRole::TitleSmall,
                    ))
                    .child(Text::body(
                        "Browse your font families or install a TrueType or OpenType font file.",
                    )),
            );
        };
        let filename = file
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let filename = filename.strip_suffix(".disabled").unwrap_or(&filename);
        stack = stack
            .child(Text::styled(&face.family, TextRole::TitleMedium))
            .child(
                Text::caption(format!(
                    "{} · {}",
                    face.style,
                    model::scope_label(file.scope)
                ))
                .color(theme.colors.text_secondary),
            );
        if let Some(family) = model
            .families
            .iter()
            .find(|family| Some(&family.name) == model.selected.as_ref())
        {
            let mut styles = HStack::new().gap(StackGap::Small);
            for key in &family.faces {
                if let Some((font, face)) = model.resolve(key) {
                    if model.filter == Filter::User && font.scope != fonts::Scope::User
                        || model.filter == Filter::System && font.scope == fonts::Scope::User
                    {
                        continue;
                    }
                    let mut button = self
                        .0
                        .button(
                            format!("{} · {}", face.style, model::scope_label(font.scope)),
                            Action::Face(key.clone()),
                        )
                        .accessibility_selected(model.face.as_ref() == Some(key));
                    if model.face.as_ref() == Some(key) {
                        button = button.style(ButtonStyle::Standard);
                    }
                    styles =
                        styles.child(button.layout().width(148.0).height(30.0).flex_shrink(0.0));
                }
            }
            stack = stack.child(
                Scroll::new(self.0.styles_scroll.clone())
                    .axis(ScrollAxis::Horizontal)
                    .content(styles)
                    .layout()
                    .height(44.0),
            );
        }
        stack = stack.child(
            TextField::with_interaction(self.0.specimen.clone())
                .placeholder("Type your preview text")
                .layout()
                .height(32.0),
        );
        stack = stack.child(
            HStack::new()
                .alignment(StackAlignment::Center)
                .gap(StackGap::Small)
                .child(self.0.button("English", Action::English))
                .child(self.0.button("Japanese", Action::Japanese))
                .child(
                    Text::caption(format!("{:.0} px", self.0.size.get()))
                        .layout()
                        .width(44.0),
                )
                .child(
                    Slider::with_interaction(
                        self.0.size.binding(),
                        self.0.size_interaction.clone(),
                    )
                    .range(12.0..=96.0)
                    .step(1.0)
                    .layout()
                    .flex_grow(1.0),
                ),
        );
        let text = self.0.specimen.value();
        let text = if text.is_empty() {
            ENGLISH.into()
        } else {
            text
        };
        let size = self.0.size.get();
        stack = stack.child(Specimen(
            Text::body(text)
                .font_family(file.preview_family(face.index))
                .font_size(size)
                .line_height(size * 1.4)
                .weight(face.weight),
        ));
        if !face.japanese {
            stack = stack.child(
                Text::caption("Missing characters may use fallback fonts.")
                    .color(theme.colors.text_secondary),
            );
        }
        if file.scope == fonts::Scope::User
            && model
                .catalogue
                .files
                .iter()
                .filter(|font| font.scope != fonts::Scope::User)
                .any(|font| font.faces.iter().any(|other| other.family == face.family))
        {
            stack=stack.child(Text::caption("A system font shares this family name. The OS interface keeps its system font.").color(theme.colors.text_secondary));
        }
        if !file.enabled {
            stack = stack.child(
                Text::caption("Disabled · Preview only; unavailable to normal font selection.")
                    .color(theme.colors.text_secondary),
            );
        }
        stack = stack
            .child(Divider::new())
            .child(info("Full name", &face.full_name, theme))
            .child(info("Collection", model::scope_label(file.scope), theme))
            .child(info(
                "Typeface",
                if face.variable {
                    "Variable"
                } else if face.monospaced {
                    "Monospaced"
                } else {
                    "Proportional"
                },
                theme,
            ))
            .child(info("Glyphs", face.glyphs.to_string(), theme))
            .child(info(
                "Format",
                if filename.ends_with(".ttc") || filename.ends_with(".otc") {
                    "Font collection"
                } else if filename.ends_with(".otf") {
                    "OpenType"
                } else {
                    "TrueType"
                },
                theme,
            ))
            .child(info(
                "File size",
                format!("{:.1} MiB", file.bytes as f64 / (1024.0 * 1024.0)),
                theme,
            ));

        if file.managed {
            stack = stack.child(
                HStack::new()
                    .gap(StackGap::Small)
                    .child(
                        self.0
                            .button(
                                if file.enabled {
                                    "Disable Font"
                                } else {
                                    "Enable Font"
                                },
                                Action::Toggle,
                            )
                            .style(ButtonStyle::Standard)
                            .enabled(!model.loading),
                    )
                    .child(
                        self.0
                            .button("Remove Font…", Action::AskRemove)
                            .enabled(!model.loading),
                    ),
            );
        } else {
            stack = stack.child(
                Text::caption(if file.scope == fonts::Scope::User {
                    "Externally managed font. Import it to manage a copy in Fonts."
                } else {
                    "System and shared font files are protected."
                })
                .color(theme.colors.text_secondary),
            );
        }
        Padding::all(theme.spacing.large).content(stack)
    }
}
impl View for Detail {
    fn measure(&self, c: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content(context.theme).measure(c, context)
    }
    fn paint(&self, b: Rect, c: &mut PaintContext<'_>) {
        self.content(c.theme).paint(b, c);
    }
    fn handle_event(&self, b: Rect, e: &ViewEvent, c: &mut EventContext<'_>) -> EventResult {
        self.content(c.theme()).handle_event(b, e, c)
    }
}

struct Specimen(Text);
impl View for Specimen {
    fn measure(&self, c: Constraints, context: &mut MeasureContext<'_>) -> Size {
        let size = self.0.measure(c, context);
        c.constrain(Size::new(size.width, size.height.max(180.0)))
    }
    fn paint(&self, b: Rect, c: &mut PaintContext<'_>) {
        self.0.paint(b, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appcore::viewkit::draw_command::{DisplayList, DrawCommand};
    use appcore::viewkit::typography::TextMeasurer;
    fn fixture() -> FontsView {
        let root = PathBuf::from(
            std::env::var_os("MOCHIOS_PREVIEW_FONTS_SYSTEM").expect("test font directory"),
        );
        let locations = Locations {
            system: vec![root],
            shared: vec![],
            user: None,
        };
        let mut model = Model::new();
        model.update(fonts::catalogue(&locations), "Inter", None);
        FontsView::with_model(model, locations)
    }
    #[test]
    fn compact_and_wide_layouts_reserve_toolbar_and_footer() {
        for width in [320.0, 719.0, 720.0, 1000.0] {
            for detail in [false, true] {
                let b = rect(12.0, 20.0, width, 600.0);
                let f = Frames::new(b, detail);
                for body in [f.sidebar, f.detail].into_iter().flatten() {
                    assert!(body.origin.y >= f.toolbar.origin.y + f.toolbar.size.height);
                    assert!(body.origin.y + body.size.height <= f.footer.origin.y);
                    assert!(body.origin.x + body.size.width <= b.origin.x + width);
                }
                assert_eq!(f.sidebar.is_some(), width >= 720.0 || !detail);
                assert_eq!(f.detail.is_some(), width >= 720.0 || detail);
            }
        }
    }
    #[test]
    fn native_components_paint_both_themes_and_emit_exact_font_references() {
        let view = fixture();
        for theme in [Theme::LIGHT, Theme::DARK] {
            for width in [360.0, 1000.0] {
                view.core.model.borrow_mut().detail_open = true;
                let mut list = DisplayList::new();
                let mut measurer = TextMeasurer::new();
                view.paint(
                    rect(0.0, 0.0, width, 680.0),
                    &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
                );
                assert!(list.commands().iter().any(|command|matches!(command,DrawCommand::DrawText{command} if command.font_family.starts_with("mochi-font:") && command.text==ENGLISH)));
                assert!(!list.commands().iter().any(|command|matches!(command,DrawCommand::DrawText{command} if command.text=="Remove Font…")));
            }
        }
    }
    #[test]
    fn search_scope_and_escape_keep_protected_fonts_safe() {
        let view = fixture();
        let initial = view.core.model.borrow().catalogue.files.len();
        view.core.search.set_value("not a real font family");
        view.core.apply();
        assert!(view.core.model.borrow().face.is_none());
        view.core.search.set_value("");
        view.core.pending.set(Some(Action::Filter(Filter::All)));
        view.core.apply();
        assert!(view.core.model.borrow().face.is_some());
        view.core.pending.set(Some(Action::AskRemove));
        view.core.apply();
        assert!(view.core.removal.borrow().is_none());
        assert_eq!(view.core.model.borrow().catalogue.files.len(), initial);
        view.core.pending.set(Some(Action::Japanese));
        view.core.apply();
        assert_eq!(view.core.specimen.value(), JAPANESE);
    }
    #[test]
    fn removal_requires_confirmation_and_escape_never_starts_a_worker() {
        let view = fixture();
        let mut catalogue = view.core.model.borrow().catalogue.clone();
        for file in &mut catalogue.files {
            file.scope = fonts::Scope::User;
            file.managed = true;
        }
        view.core.model.borrow_mut().update(catalogue, "", None);
        let count = view.core.model.borrow().catalogue.files.len();
        view.core.pending.set(Some(Action::AskRemove));
        view.core.apply();
        assert!(view.core.removal.borrow().is_some());
        assert!(view.core.receiver.borrow().is_none());
        let theme = Theme::LIGHT;
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        view.handle_event(
            rect(0.0, 0.0, 1000.0, 680.0),
            &ViewEvent::KeyPressed {
                key: appcore::viewkit::platform::Key::Escape,
                modifiers: Default::default(),
            },
            &mut context,
        );
        assert!(view.core.removal.borrow().is_none());
        assert!(view.core.receiver.borrow().is_none());
        assert_eq!(view.core.model.borrow().catalogue.files.len(), count);
    }
}
