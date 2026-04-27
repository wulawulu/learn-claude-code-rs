use std::{
    fs,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub path: PathBuf,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub path: PathBuf,
    pub existed: bool,
}

#[derive(Clone, Debug)]
pub struct StoreRoot {
    root: PathBuf,
}

impl StoreRoot {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create store root {}", root.display()))?;
        let root = root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize store root {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn file<T>(&self, relative: &str) -> Result<Store<T>> {
        Ok(Store::new(self.resolve(relative, true)?))
    }

    pub fn collection<T>(&self, relative_dir: &str) -> Result<CollectionStore<T>> {
        let dir = self.resolve(relative_dir, true)?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create collection {}", dir.display()))?;
        Ok(CollectionStore::new(dir))
    }

    fn resolve(&self, relative: &str, allow_missing: bool) -> Result<PathBuf> {
        let path = Path::new(relative);
        if path.is_absolute() {
            anyhow::bail!("store path must be relative: {relative}");
        }

        let candidate = self.root.join(path);
        let resolved = if candidate.exists() || !allow_missing {
            candidate
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", candidate.display()))?
        } else {
            let parent = candidate.parent().context("store path has no parent")?;
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
            let parent = parent
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", parent.display()))?;
            let file_name = candidate
                .file_name()
                .context("store path has no file name")?;
            parent.join(file_name)
        };

        if !resolved.starts_with(&self.root) {
            anyhow::bail!("store path escapes root: {relative}");
        }
        Ok(resolved)
    }
}

#[derive(Clone, Debug)]
pub struct Store<T> {
    path: PathBuf,
    _marker: PhantomData<T>,
}

impl<T> Store<T> {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            _marker: PhantomData,
        }
    }
}

impl<T> Store<T>
where
    T: Serialize + DeserializeOwned,
{
    pub fn read(&self) -> Result<T> {
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", self.path.display()))
    }

    pub fn write(&self, value: &T) -> Result<WriteOutcome> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = format!("{}\n", serde_json::to_string_pretty(value)?);
        fs::write(&self.path, &content)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        Ok(WriteOutcome {
            path: self.path.clone(),
            bytes: content.len(),
        })
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> Result<R>) -> Result<R> {
        let mut value = self.read()?;
        let result = f(&mut value)?;
        self.write(&value)?;
        Ok(result)
    }

    pub fn append(&self, value: &T) -> Result<WriteOutcome> {
        use std::io::Write;

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = format!("{}\n", serde_json::to_string(value)?);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("failed to append {}", self.path.display()))?;
        Ok(WriteOutcome {
            path: self.path.clone(),
            bytes: content.len(),
        })
    }

    pub fn read_all(&self) -> Result<Vec<T>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .with_context(|| format!("failed to parse line in {}", self.path.display()))
            })
            .collect()
    }

    pub fn delete(&self) -> Result<DeleteOutcome> {
        if !self.path.exists() {
            return Ok(DeleteOutcome {
                path: self.path.clone(),
                existed: false,
            });
        }
        fs::remove_file(&self.path)
            .with_context(|| format!("failed to delete {}", self.path.display()))?;
        Ok(DeleteOutcome {
            path: self.path.clone(),
            existed: true,
        })
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }
}

#[derive(Clone, Debug)]
pub struct CollectionStore<T> {
    dir: PathBuf,
    _marker: PhantomData<T>,
}

impl<T> CollectionStore<T> {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            _marker: PhantomData,
        }
    }
}

impl<T> CollectionStore<T>
where
    T: Serialize + DeserializeOwned,
{
    pub fn read(&self, key: &str) -> Result<T> {
        Store::new(self.path_for(key)?).read()
    }

    pub fn write(&self, key: &str, value: &T) -> Result<WriteOutcome> {
        Store::new(self.path_for(key)?).write(value)
    }

    pub fn append(&self, key: &str, value: &T) -> Result<WriteOutcome> {
        Store::new(self.path_for(key)?).append(value)
    }

    pub fn read_all_from(&self, key: &str) -> Result<Vec<T>> {
        Store::new(self.path_for(key)?).read_all()
    }

    pub fn delete(&self, key: &str) -> Result<DeleteOutcome> {
        Store::<T>::new(self.path_for(key)?).delete()
    }

    pub fn list(&self) -> Result<Vec<T>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut values = Vec::new();
        for entry in fs::read_dir(&self.dir)
            .with_context(|| format!("failed to read {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            values.push(Store::new(path).read()?);
        }
        Ok(values)
    }

    pub fn exists(&self, key: &str) -> bool {
        self.path_for(key).is_ok_and(|path| path.exists())
    }

    fn path_for(&self, key: &str) -> Result<PathBuf> {
        if key.contains('/') || key.contains('\\') || key == "." || key == ".." {
            anyhow::bail!("invalid collection key: {key}");
        }
        Ok(self.dir.join(format!("{key}.json")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
    struct Item {
        value: String,
    }

    #[test]
    fn store_round_trips_json() {
        let root = StoreRoot::new(
            std::env::temp_dir().join(format!("sfull-store-test-{}", std::process::id())),
        )
        .unwrap();
        let store = root.file::<Item>("items/one.json").unwrap();

        store
            .write(&Item {
                value: "hello".to_string(),
            })
            .unwrap();

        assert_eq!(
            store.read().unwrap(),
            Item {
                value: "hello".to_string()
            }
        );
    }
}
