//! Providers: where bundle files come from.
//!
//! A provider lists the bundle files it offers and reads one on request. The registry never
//! walks a filesystem itself: a folder, a VFS, and later a package are all the same trait, and
//! the order they're registered in is the only precedence there is.
//!
//! Every provider lays files out the way `OpenMW` does (`Manager::readLangData` in
//! `components/l10n/manager.cpp` opens `l10n/<context>/<locale>.yaml`): one folder per bundle, one
//! file per locale, here `<bundle>/<locale>.toml`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// The extension of a bundle file.
pub const EXTENSION: &str = "toml";

/// One bundle file a provider offers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resource {
    /// The bundle's name: the folder.
    pub bundle: String,
    /// The locale as the file spells it: the file stem, not yet validated.
    pub locale: String,
    /// The path, as diagnostics and provenance show it and as [`Provider::read`] takes it back.
    pub path: String,
}

/// A provider that couldn't list or read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProviderError {
    /// The path that failed, when one did.
    pub path: Option<String>,
    /// Why.
    pub reason: String,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{path}: {}", self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

impl std::error::Error for ProviderError {}

/// A source of bundle files.
///
/// Implementations are `Send + Sync` so a registry can be shared between threads. A provider
/// must not offer the same bundle and locale twice; the registry reports that as a collision
/// instead of picking one.
pub trait Provider: Send + Sync {
    /// The provider's name, for provenance and diagnostics.
    fn name(&self) -> &str;

    /// Every bundle file the provider offers right now. The order doesn't matter.
    ///
    /// # Errors
    ///
    /// Returns the provider's error when the files can't be listed; the registry then reloads nothing.
    fn resources(&self) -> Result<Vec<Resource>, ProviderError>;

    /// The text of one of the files [`resources`](Self::resources) listed.
    ///
    /// # Errors
    ///
    /// Returns the provider's error when the file can't be read; the bundle then keeps its last good generation.
    fn read(&self, resource: &Resource) -> Result<String, ProviderError>;
}

/// Splits `<bundle>/<locale>.toml` into its bundle and locale.
fn split_resource(relative: &str) -> Option<(&str, &str)> {
    let (bundle, file) = relative.split_once('/')?;
    let locale = file.strip_suffix(EXTENSION)?.strip_suffix('.')?;
    if bundle.is_empty() || locale.is_empty() || file.contains('/') {
        return None;
    }
    Some((bundle, locale))
}

/// A folder of bundles on the host filesystem: `<root>/<bundle>/<locale>.toml`.
///
/// This is the provider for a package source folder and for tests. It reads only under the root
/// it was given; nothing else on the host is looked at.
#[derive(Clone, Debug)]
pub struct DirectoryProvider {
    name: String,
    root: PathBuf,
}

impl DirectoryProvider {
    /// A provider over `root`, which holds one folder per bundle.
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root: root.into(),
        }
    }

    /// The folder the provider reads.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> ProviderError {
    ProviderError {
        path: Some(path.display().to_string()),
        reason: error.to_string(),
    }
}

impl Provider for DirectoryProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn resources(&self) -> Result<Vec<Resource>, ProviderError> {
        let mut resources = Vec::new();
        for bundle in std::fs::read_dir(&self.root).map_err(|error| io_error(&self.root, &error))? {
            let bundle = bundle.map_err(|error| io_error(&self.root, &error))?;
            let bundle_path = bundle.path();
            if !bundle_path.is_dir() {
                continue;
            }
            // A folder or file whose name isn't UTF-8 can't name a bundle or a locale.
            let Some(bundle_name) = bundle.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            for file in std::fs::read_dir(&bundle_path).map_err(|error| io_error(&bundle_path, &error))? {
                let file = file.map_err(|error| io_error(&bundle_path, &error))?;
                let path = file.path();
                let Some(file_name) = file.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let relative = format!("{bundle_name}/{file_name}");
                let Some((_, locale)) = split_resource(&relative) else {
                    continue;
                };
                if !path.is_file() {
                    continue;
                }
                resources.push(Resource {
                    bundle: bundle_name.clone(),
                    locale: locale.to_owned(),
                    path: path.display().to_string(),
                });
            }
        }
        resources.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(resources)
    }

    fn read(&self, resource: &Resource) -> Result<String, ProviderError> {
        let path = Path::new(&resource.path);
        std::fs::read_to_string(path).map_err(|error| io_error(path, &error))
    }
}

/// A virtual filesystem as a provider sees it: a flat list of `/`-separated paths and their text.
///
/// This is the shape of jess's VFS (and `OpenMW`'s): the caller adapts whatever resolves its
/// layers into one winning file per path, and localization reads through it like everything else.
pub trait VfsSource: Send + Sync {
    /// Every file path in the VFS, `/`-separated, in any order.
    ///
    /// # Errors
    ///
    /// Returns the source's error when the paths can't be listed.
    fn paths(&self) -> Result<Vec<String>, ProviderError>;

    /// The text of the file at `path`.
    ///
    /// # Errors
    ///
    /// Returns the source's error when the file can't be read.
    fn read(&self, path: &str) -> Result<String, ProviderError>;
}

/// Bundles under a folder of a [`VfsSource`]: `<prefix>/<bundle>/<locale>.toml`.
#[derive(Clone, Debug)]
pub struct VfsProvider<S> {
    name: String,
    prefix: String,
    source: S,
}

impl<S: VfsSource> VfsProvider<S> {
    /// A provider over the files under `prefix` (`l10n`, say) in `source`. An empty prefix takes
    /// bundles from the VFS root.
    pub fn new(name: impl Into<String>, prefix: &str, source: S) -> Self {
        let mut prefix = prefix.trim_matches('/').to_owned();
        if !prefix.is_empty() {
            prefix.push('/');
        }
        Self {
            name: name.into(),
            prefix,
            source,
        }
    }

    /// The VFS the provider reads through.
    pub fn source(&self) -> &S {
        &self.source
    }
}

impl<S: VfsSource> Provider for VfsProvider<S> {
    fn name(&self) -> &str {
        &self.name
    }

    fn resources(&self) -> Result<Vec<Resource>, ProviderError> {
        let mut resources = Vec::new();
        for path in self.source.paths()? {
            let Some((bundle, locale)) = path.strip_prefix(&self.prefix).and_then(split_resource) else {
                continue;
            };
            resources.push(Resource {
                bundle: bundle.to_owned(),
                locale: locale.to_owned(),
                path: path.clone(),
            });
        }
        resources.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(resources)
    }

    fn read(&self, resource: &Resource) -> Result<String, ProviderError> {
        self.source.read(&resource.path)
    }
}

/// Files held in memory: a [`VfsSource`] for tests, embedded defaults, and tools.
///
/// Clones share one set of files, so a test can keep a handle, change a file, and reload the
/// registry that owns the other.
#[derive(Clone, Debug, Default)]
pub struct MemoryFiles {
    files: Arc<Mutex<BTreeMap<String, String>>>,
}

impl MemoryFiles {
    /// No files.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the file at `path`.
    pub fn insert(&self, path: impl Into<String>, text: impl Into<String>) {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.into(), text.into());
    }

    /// Removes the file at `path`, if there is one.
    pub fn remove(&self, path: &str) {
        self.files.lock().unwrap_or_else(PoisonError::into_inner).remove(path);
    }
}

impl VfsSource for MemoryFiles {
    fn paths(&self) -> Result<Vec<String>, ProviderError> {
        Ok(self
            .files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect())
    }

    fn read(&self, path: &str) -> Result<String, ProviderError> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned()
            .ok_or_else(|| ProviderError {
                path: Some(path.to_owned()),
                reason: "no such file".to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_paths_split() {
        assert_eq!(split_resource("common/en.toml"), Some(("common", "en")));
        assert_eq!(split_resource("jess.common/pt-BR.toml"), Some(("jess.common", "pt-BR")));
        assert_eq!(split_resource("common/en.yaml"), None);
        assert_eq!(split_resource("common/deeper/en.toml"), None);
        assert_eq!(split_resource("en.toml"), None);
        assert_eq!(split_resource("common/.toml"), None);
    }

    #[test]
    fn vfs_provider_lists_only_its_prefix() {
        let files = MemoryFiles::new();
        files.insert("l10n/common/en.toml", "hello = 'Hello'");
        files.insert("l10n/common/de.toml", "hello = 'Hallo'");
        files.insert("l10n/common/notes.txt", "not a bundle");
        files.insert("meshes/common/en.toml", "not under the prefix");
        let provider = VfsProvider::new("memory", "l10n", files);
        let resources = provider.resources().unwrap();
        let paths: Vec<&str> = resources.iter().map(|resource| resource.path.as_str()).collect();
        assert_eq!(paths, ["l10n/common/de.toml", "l10n/common/en.toml"]);
        assert_eq!(resources[0].bundle, "common");
        assert_eq!(resources[0].locale, "de");
        assert_eq!(provider.read(&resources[1]).unwrap(), "hello = 'Hello'");
    }
}
