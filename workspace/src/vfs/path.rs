/// A mount-relative virtual path, always absolute within its mount
/// (leading `/`, no trailing `/` except for the root `/`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountPath(String);

impl MountPath {
    /// The path of `name` inside this directory, without the `//` a root join makes.
    pub fn child(&self, name: &str) -> Self {
        if self.is_root() {
            Self::new(format!("/{name}"))
        } else {
            Self::new(format!("{}/{}", self.as_str(), name))
        }
    }

    pub fn new(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        let trimmed = s.trim_end_matches('/');
        if trimmed.is_empty() {
            MountPath("/".to_string())
        } else if let Some(stripped) = trimmed.strip_prefix('/') {
            MountPath(format!("/{stripped}"))
        } else {
            MountPath(format!("/{trimmed}"))
        }
    }

    pub fn root() -> Self {
        MountPath("/".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }
}
