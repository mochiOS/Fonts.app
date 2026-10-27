use appcore::fonts::{Catalogue, Face, FontFile, Scope};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FaceKey {
    pub scope: Scope,
    pub digest: String,
    pub index: u32,
    pub storage: String,
}
impl FaceKey {
    pub fn new(file: &FontFile, face: &Face) -> Self {
        Self {
            scope: file.scope,
            digest: file.digest.clone(),
            index: face.index,
            storage: file
                .path
                .to_string_lossy()
                .strip_suffix(".disabled")
                .unwrap_or(&file.path.to_string_lossy())
                .to_owned(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Filter {
    All,
    User,
    System,
}
#[derive(Clone, Debug)]
pub struct Family {
    pub name: String,
    pub faces: Vec<FaceKey>,
}
pub struct Model {
    pub catalogue: Catalogue,
    pub families: Vec<Family>,
    pub selected: Option<String>,
    pub face: Option<FaceKey>,
    pub filter: Filter,
    pub detail_open: bool,
    pub loading: bool,
    pub notice: String,
}
impl Model {
    pub fn new() -> Self {
        Self {
            catalogue: Default::default(),
            families: vec![],
            selected: None,
            face: None,
            filter: Filter::All,
            detail_open: false,
            loading: false,
            notice: String::new(),
        }
    }
    pub fn resolve(&self, key: &FaceKey) -> Option<(&FontFile, &Face)> {
        let file = self.catalogue.files.iter().find(|file| {
            file.scope == key.scope
                && file.digest == key.digest
                && file
                    .path
                    .to_string_lossy()
                    .strip_suffix(".disabled")
                    .unwrap_or(&file.path.to_string_lossy())
                    == key.storage
        })?;
        Some((
            file,
            file.faces.iter().find(|face| face.index == key.index)?,
        ))
    }
    fn accepts(&self, key: &FaceKey) -> bool {
        match self.filter {
            Filter::All => true,
            Filter::User => key.scope == Scope::User,
            Filter::System => key.scope != Scope::User,
        }
    }
    pub fn visible(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.families
            .iter()
            .enumerate()
            .filter(|(_, family)| {
                family.faces.iter().any(|key| self.accepts(key))
                    && (query.is_empty()
                        || family.name.to_lowercase().contains(&query)
                        || family
                            .faces
                            .iter()
                            .filter(|k| self.accepts(k))
                            .filter_map(|key| self.resolve(key))
                            .any(|(_, face)| face.full_name.to_lowercase().contains(&query)))
            })
            .map(|(index, _)| index)
            .collect()
    }
    pub fn reconcile(&mut self, query: &str) {
        let visible = self.visible(query);
        if !visible
            .iter()
            .any(|i| self.selected.as_deref() == Some(self.families[*i].name.as_str()))
        {
            self.selected = visible.first().map(|i| self.families[*i].name.clone());
            self.face = None;
        }
        let family = self
            .selected
            .as_ref()
            .and_then(|name| self.families.iter().find(|f| &f.name == name));
        if !self.face.as_ref().is_some_and(|key| {
            family.is_some_and(|family| family.faces.contains(key)) && self.accepts(key)
        }) {
            self.face = family
                .and_then(|family| {
                    family
                        .faces
                        .iter()
                        .filter(|key| self.accepts(key))
                        .min_by_key(|key| {
                            self.resolve(key).map(|(file, face)| {
                                (
                                    !file.enabled,
                                    face.weight.abs_diff(400),
                                    face.italic,
                                    key.scope,
                                )
                            })
                        })
                })
                .cloned();
        }
        if self.face.is_none() {
            self.detail_open = false;
        }
    }
    pub fn update(&mut self, catalogue: Catalogue, query: &str, preferred: Option<FaceKey>) {
        self.catalogue = catalogue;
        let mut groups = BTreeMap::<String, Vec<FaceKey>>::new();
        for file in &self.catalogue.files {
            for face in &file.faces {
                groups
                    .entry(face.family.clone())
                    .or_default()
                    .push(FaceKey::new(file, face));
            }
        }
        self.families = groups
            .into_iter()
            .map(|(name, mut faces)| {
                faces.sort_by_key(|key| {
                    self.resolve(key)
                        .map(|(file, face)| (face.weight, face.italic, !file.enabled, key.scope))
                });
                faces.dedup();
                Family { name, faces }
            })
            .collect();
        self.families
            .sort_by_cached_key(|family| family.name.to_lowercase());
        if let Some(key) = preferred {
            if let Some((_, face)) = self.resolve(&key) {
                self.selected = Some(face.family.clone());
                self.face = Some(key);
            }
        }
        self.loading = false;
        self.reconcile(query);
    }
    pub fn select(&mut self, name: &str, query: &str) {
        self.selected = Some(name.into());
        self.face = None;
        self.reconcile(query);
        self.detail_open = self.face.is_some();
    }
    pub fn selected_face(&self) -> Option<(&FontFile, &Face)> {
        self.face.as_ref().and_then(|key| self.resolve(key))
    }
}
pub fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::System => "System",
        Scope::Shared => "Shared",
        Scope::User => "User",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Catalogue {
        let data = [0u8; 8];
        let faces = vec![Face {
            index: 0,
            family: "Inter".into(),
            style: "Regular".into(),
            full_name: "Inter Regular".into(),
            postscript_name: "Inter-Regular".into(),
            weight: 400,
            italic: false,
            monospaced: false,
            variable: true,
            glyphs: 500,
            japanese: false,
        }];
        Catalogue {
            files: vec![
                FontFile {
                    path: "/system/Inter.ttf".into(),
                    digest: "system".into(),
                    scope: Scope::System,
                    enabled: true,
                    bytes: data.len() as u64,
                    faces: faces.clone(),
                    managed: false,
                },
                FontFile {
                    path: "/user/Inter.ttf".into(),
                    digest: "user".into(),
                    scope: Scope::User,
                    enabled: true,
                    bytes: data.len() as u64,
                    faces,
                    managed: true,
                },
            ],
            warnings: vec![],
        }
    }
    #[test]
    fn filtering_and_refresh_keep_face_identity_and_clear_missing_results() {
        let mut model = Model::new();
        model.update(fixture(), "", None);
        assert_eq!(model.families.len(), 1);
        assert_eq!(model.face.as_ref().unwrap().scope, Scope::System);
        model.filter = Filter::User;
        model.reconcile("Inter");
        let selected = model.face.clone();
        assert_eq!(selected.as_ref().unwrap().scope, Scope::User);
        let mut catalogue = fixture();
        catalogue.files[1].enabled = false;
        catalogue.files[1].path = "/user/Inter.ttf.disabled".into();
        model.update(catalogue, "Inter", None);
        assert_eq!(model.face, selected);
        model.reconcile("not a font");
        assert!(model.face.is_none());
        assert!(model.visible("not a font").is_empty());
    }
    #[test]
    fn removing_the_selected_user_font_does_not_remove_its_system_family() {
        let mut model = Model::new();
        model.update(fixture(), "", None);
        model.filter = Filter::User;
        model.reconcile("");
        let mut catalogue = fixture();
        catalogue.files.retain(|file| file.scope == Scope::System);
        model.update(catalogue, "", None);
        assert!(model.face.is_none());
        model.filter = Filter::All;
        model.reconcile("");
        assert_eq!(model.face.as_ref().unwrap().scope, Scope::System);
    }
    #[test]
    fn identical_external_and_managed_copies_remain_distinct() {
        let mut catalogue = fixture();
        catalogue.files.retain(|file| file.scope == Scope::User);
        let mut external = catalogue.files[0].clone();
        external.path = "/user/external.ttf".into();
        external.managed = false;
        catalogue.files.insert(0, external);
        let managed = &catalogue.files[1];
        let key = FaceKey::new(managed, &managed.faces[0]);
        let mut model = Model::new();
        model.update(catalogue, "", Some(key.clone()));
        assert_eq!(model.face, Some(key));
        assert!(model.selected_face().unwrap().0.managed);
        assert_eq!(model.families[0].faces.len(), 2);
    }
}
