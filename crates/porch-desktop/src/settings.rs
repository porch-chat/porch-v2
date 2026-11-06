use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub font_size: f32,
    pub members: bool,
    pub origin: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 15.,
            members: true,
            origin: "http://127.0.0.1:8080".into(),
        }
    }
}
impl Settings {
    pub fn load(root: &Path) -> Self {
        let mut s: Self = std::fs::read(root.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        s.font_size = s.font_size.clamp(12., 22.);
        s.origin = canonical_origin(&s.origin).unwrap_or_else(|| Self::default().origin);
        s
    }
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        // Validate before serializing: an origin with URL credentials/query
        // must never persist, even when a login attempt itself would reject it.
        let origin = canonical_origin(&self.origin).ok_or(std::io::ErrorKind::InvalidInput)?;
        let mut safe = self.clone();
        safe.origin = origin;
        // No credentials. Unix rename replaces atomically; Windows has a brief
        // remove/rename gap, recovered to defaults by load if interrupted.
        let tmp = root.join("settings.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&safe)?)?;
        let path = root.join("settings.json");
        // Windows rename does not replace existing files.
        if cfg!(windows) && path.exists() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(tmp, path)
    }
}
fn canonical_origin(origin: &str) -> Option<String> {
    porch_core::Api::new(origin, "origin-validation".into())
        .ok()
        .map(|api| api.origin().to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_clamps_sizes_and_recovers_corrupt_preferences() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings {
            font_size: 100.,
            members: false,
            origin: "https://porch.example".into(),
        };
        settings.save(root.path()).unwrap();
        let loaded = Settings::load(root.path());
        assert_eq!(loaded.font_size, 22.);
        assert!(!loaded.members);
        assert_eq!(loaded.origin, settings.origin);
        std::fs::write(root.path().join("settings.json"), b"broken").unwrap();
        assert_eq!(Settings::load(root.path()).font_size, 15.);
    }
    #[test]
    fn rejects_sensitive_urls_before_writing_and_recovers_on_load() {
        let root = tempfile::tempdir().unwrap();
        for origin in [
            "https://person:secret@porch.example",
            "https://porch.example/?token=secret",
            "https://porch.example/#secret",
        ] {
            let s = Settings {
                origin: origin.into(),
                ..Default::default()
            };
            assert!(s.save(root.path()).is_err());
            assert!(!root.path().join("settings.json").exists());
            // Older/hand-edited preferences are not displayed again.
            std::fs::write(
                root.path().join("settings.json"),
                serde_json::to_vec(&s).unwrap(),
            )
            .unwrap();
            assert_eq!(
                Settings::load(root.path()).origin,
                Settings::default().origin
            );
            std::fs::remove_file(root.path().join("settings.json")).unwrap();
        }
    }
    #[test]
    fn no_secret_fields() {
        let s = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!s.contains("token"));
        assert!(!s.contains("password"));
    }
}
