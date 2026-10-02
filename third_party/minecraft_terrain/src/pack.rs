//! External client assets. Stack order is low to high priority; no gameplay data is read here.
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const FORMAT: (u32, u32) = (97, 1);
const MAX_ENTRY: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ResourceId {
    pub namespace: String,
    pub path: String,
}
impl ResourceId {
    pub fn parse(raw: &str) -> Result<Self> {
        let (namespace, path) = raw.split_once(':').unwrap_or(("minecraft", raw));
        if namespace.is_empty()
            || !namespace
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
            || path.is_empty()
            || path.starts_with('/')
            || path.ends_with('/')
            || path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || !path
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_./-".contains(&c))
        {
            bail!("invalid resource identifier: {raw}");
        }
        Ok(Self {
            namespace: namespace.into(),
            path: path.into(),
        })
    }
    pub fn asset(&self, category: &str, suffix: &str) -> String {
        format!("assets/{}/{category}/{}{suffix}", self.namespace, self.path)
    }
    pub fn key(&self) -> String {
        format!("{}:{}", self.namespace, self.path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u32, u32);
fn version(v: &Value, upper: bool) -> Result<Version> {
    if let Some(n) = v.as_u64() {
        return Ok(Version(u32::try_from(n)?, if upper { u32::MAX } else { 0 }));
    }
    let a = v
        .as_array()
        .ok_or_else(|| anyhow!("format version must be integer or [major,minor]"))?;
    if a.is_empty() || a.len() > 2 {
        bail!("format version needs one or two components");
    }
    let major = u32::try_from(
        a[0].as_u64()
            .ok_or_else(|| anyhow!("invalid major format"))?,
    )?;
    let minor = if a.len() == 2 {
        u32::try_from(
            a[1].as_u64()
                .ok_or_else(|| anyhow!("invalid minor format"))?,
        )?
    } else if upper {
        u32::MAX
    } else {
        0
    };
    Ok(Version(major, minor))
}

#[derive(Debug)]
struct Filter {
    namespace: Option<Regex>,
    path: Option<Regex>,
}
impl Filter {
    fn new(v: &Value) -> Result<Self> {
        let obj = v
            .as_object()
            .ok_or_else(|| anyhow!("filter entry must be object"))?;
        let compile = |key| -> Result<Option<Regex>> {
            obj.get(key)
                .map(|v| -> Result<Regex> {
                    let s = v
                        .as_str()
                        .ok_or_else(|| anyhow!("filter {key} must be regex string"))?;
                    Ok(Regex::new(&format!(r"\A(?:{s})\z"))?)
                })
                .transpose()
        };
        Ok(Self {
            namespace: compile("namespace")?,
            path: compile("path")?,
        })
    }
    fn matches(&self, id: &ResourceId, path: &str) -> bool {
        self.namespace
            .as_ref()
            .map_or(true, |r| r.is_match(&id.namespace))
            && self.path.as_ref().map_or(true, |r| r.is_match(path))
    }
}

#[derive(Clone, Debug)]
enum Entry {
    Directory(PathBuf),
    Zip(usize),
}

#[derive(Debug)]
pub struct Pack {
    pub source: PathBuf,
    pub description: String,
    pub metadata_hash: String,
    raw: HashMap<String, Entry>,
    entries: HashMap<String, Entry>,
    filters: Vec<Filter>,
    zip: bool,
}
impl Pack {
    pub fn open(source: impl AsRef<Path>) -> Result<Self> {
        let source = source.as_ref().to_path_buf();
        let zip = source.is_file();
        if !zip && !source.is_dir() {
            bail!("pack does not exist: {}", source.display());
        }
        let mut raw = HashMap::new();
        if zip {
            let mut archive = zip::ZipArchive::new(File::open(&source)?)?;
            let mut total = 0u64;
            for i in 0..archive.len() {
                let entry = archive.by_index(i)?;
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().replace('\\', "/");
                validate_path(&name)?;
                if let Some(mode) = entry.unix_mode() {
                    if mode & 0o170000 == 0o120000 {
                        bail!("ZIP symlink rejected: {name}");
                    }
                }
                if entry.size() > MAX_ENTRY {
                    bail!("ZIP entry too large: {name}");
                }
                total = total
                    .checked_add(entry.size())
                    .ok_or_else(|| anyhow!("archive size overflow"))?;
                if total > MAX_ARCHIVE {
                    bail!("ZIP unpacked content exceeds 2 GiB");
                }
                if raw.insert(name.clone(), Entry::Zip(i)).is_some() {
                    bail!("duplicate ZIP entry: {name}");
                }
            }
        } else {
            collect_directory(&source, &source, &mut raw)?;
        }
        let meta_bytes = read_entry(
            &source,
            zip,
            raw.get("pack.mcmeta")
                .ok_or_else(|| anyhow!("pack.mcmeta missing"))?,
        )?;
        let metadata_hash = format!("{:x}", Sha256::digest(&meta_bytes));
        let meta: Value = serde_json::from_slice(&meta_bytes).context("malformed pack.mcmeta")?;
        let p = meta
            .get("pack")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("pack.mcmeta needs pack object"))?;
        let min = version(
            p.get("min_format")
                .ok_or_else(|| anyhow!("missing min_format"))?,
            false,
        )?;
        let max = version(
            p.get("max_format")
                .ok_or_else(|| anyhow!("missing max_format"))?,
            true,
        )?;
        if min > max || Version(FORMAT.0, FORMAT.1) < min || Version(FORMAT.0, FORMAT.1) > max {
            bail!("pack format {min:?}..{max:?} does not include 97.1");
        }
        let description = p
            .get("description")
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default();
        let mut filters = Vec::new();
        if let Some(filter) = meta.get("filter") {
            let block = filter
                .get("block")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("filter.block must be array"))?;
            for v in block {
                filters.push(Filter::new(v)?);
            }
        }
        let mut entries = HashMap::new();
        for (key, value) in &raw {
            if key.starts_with("assets/") {
                entries.insert(key.clone(), value.clone());
            }
        }
        if let Some(overlays) = meta.get("overlays") {
            let overlays = overlays
                .get("entries")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("overlays.entries must be array"))?;
            for overlay in overlays {
                let dir = overlay
                    .get("directory")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("overlay.directory missing"))?;
                if !dir
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
                    || dir.is_empty()
                {
                    bail!("invalid overlay directory: {dir}");
                }
                let min = version(
                    overlay
                        .get("min_format")
                        .ok_or_else(|| anyhow!("overlay min_format missing"))?,
                    false,
                )?;
                let max = version(
                    overlay
                        .get("max_format")
                        .ok_or_else(|| anyhow!("overlay max_format missing"))?,
                    true,
                )?;
                if min > max {
                    bail!("overlay invalid range");
                }
                if Version(FORMAT.0, FORMAT.1) >= min && Version(FORMAT.0, FORMAT.1) <= max {
                    let prefix = format!("{dir}/assets/");
                    for (key, value) in &raw {
                        if key.starts_with(&prefix) {
                            entries.insert(key[dir.len() + 1..].into(), value.clone());
                        }
                    }
                }
            }
        }
        Ok(Self {
            source,
            description,
            metadata_hash,
            raw,
            entries,
            filters,
            zip,
        })
    }
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.entries
            .get(path)
            .map(|e| read_entry(&self.source, self.zip, e))
            .transpose()
    }
    fn blocks(&self, id: &ResourceId, path: &str) -> bool {
        self.filters.iter().any(|f| f.matches(id, path))
    }
    pub fn namespaces(&self) -> Vec<String> {
        let mut n: Vec<_> = self
            .entries
            .keys()
            .filter_map(|k| {
                k.strip_prefix("assets/")?
                    .split_once('/')
                    .map(|(n, _)| n.to_string())
            })
            .collect();
        n.sort();
        n.dedup();
        n
    }
    pub fn content_hash(&self) -> Result<String> {
        hash_contents(&self.source, self.zip, &self.raw)
    }
    fn list(&self, prefix: &str) -> Vec<String> {
        self.entries
            .keys()
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect()
    }
}

fn validate_path(path: &str) -> Result<()> {
    if path.starts_with('/')
        || path.contains(':')
        || path
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
    {
        bail!("unsafe pack path: {path}");
    }
    Ok(())
}
fn collect_directory(root: &Path, dir: &Path, out: &mut HashMap<String, Entry>) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let ty = item.file_type()?;
        if ty.is_symlink() {
            bail!("pack symlink rejected: {}", item.path().display());
        }
        if ty.is_dir() {
            collect_directory(root, &item.path(), out)?;
        } else if ty.is_file() {
            let rel = item
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            validate_path(&rel)?;
            if item.metadata()?.len() > MAX_ENTRY {
                bail!("pack resource too large: {rel}");
            }
            out.insert(rel, Entry::Directory(item.path()));
        }
    }
    Ok(())
}
fn read_entry(source: &Path, zip: bool, entry: &Entry) -> Result<Vec<u8>> {
    match entry {
        Entry::Directory(path) => {
            Ok(fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        }
        Entry::Zip(index) if zip => {
            let mut archive = zip::ZipArchive::new(File::open(source)?)?;
            let file = archive.by_index(*index)?;
            let mut bytes = Vec::with_capacity(file.size() as usize);
            file.take(MAX_ENTRY + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_ENTRY {
                bail!("ZIP inflated resource exceeds limit");
            }
            Ok(bytes)
        }
        _ => bail!("invalid pack entry"),
    }
}
fn hash_contents(source: &Path, zip: bool, entries: &HashMap<String, Entry>) -> Result<String> {
    let mut hasher = Sha256::new();
    if zip {
        let mut file = File::open(source)?;
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
    } else {
        let mut names: Vec<_> = entries.keys().collect();
        names.sort();
        for name in names {
            hasher.update(name.as_bytes());
            hasher.update([0]);
            let mut file = File::open(match &entries[name] {
                Entry::Directory(path) => path,
                _ => unreachable!(),
            })?;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug)]
pub struct PackStack {
    sources: Vec<PathBuf>,
    packs: Vec<Pack>,
    cache: RefCell<HashMap<String, Option<Vec<u8>>>>,
    pub generation: u64,
}
impl PackStack {
    pub fn open(sources: Vec<PathBuf>) -> Result<Self> {
        let packs = sources.iter().map(Pack::open).collect::<Result<Vec<_>>>()?;
        Ok(Self {
            sources,
            packs,
            cache: RefCell::new(HashMap::new()),
            generation: 0,
        })
    }
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }
    pub fn packs(&self) -> &[Pack] {
        &self.packs
    }
    pub fn reload(&mut self) -> Result<()> {
        let packs = self
            .sources
            .iter()
            .map(Pack::open)
            .collect::<Result<Vec<_>>>()?;
        self.packs = packs;
        self.cache.borrow_mut().clear();
        self.generation += 1;
        Ok(())
    }
    pub fn set_sources(&mut self, sources: Vec<PathBuf>) -> Result<()> {
        let packs = sources.iter().map(Pack::open).collect::<Result<Vec<_>>>()?;
        self.sources = sources;
        self.packs = packs;
        self.cache.borrow_mut().clear();
        self.generation += 1;
        Ok(())
    }
    pub fn get(&self, id: &ResourceId, path: &str) -> Result<Option<Vec<u8>>> {
        let full = format!("assets/{}/{}", id.namespace, path);
        if let Some(cached) = self.cache.borrow().get(&full) {
            return Ok(cached.clone());
        }
        for pack in self.packs.iter().rev() {
            if let Some(bytes) = pack.read(&full)? {
                self.cache.borrow_mut().insert(full, Some(bytes.clone()));
                return Ok(Some(bytes));
            }
            if pack.blocks(id, path) {
                self.cache.borrow_mut().insert(full, None);
                return Ok(None);
            }
        }
        self.cache.borrow_mut().insert(full, None);
        Ok(None)
    }
    pub fn json(&self, id: &ResourceId, path: &str) -> Result<Option<Value>> {
        self.get(id, path)?
            .map(|bytes| {
                serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON {path}"))
            })
            .transpose()
    }
    pub fn texture(&self, id: &ResourceId) -> Result<Option<Vec<u8>>> {
        self.get(id, &format!("textures/{}.png", id.path))
    }
    pub fn animation(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("textures/{}.png.mcmeta", id.path))
    }
    pub fn blockstate(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("blockstates/{}.json", id.path))
    }
    pub fn model(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("models/{}.json", id.path))
    }
    pub fn atlas(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.merged_array(id, &format!("atlases/{}.json", id.path), "sources")
    }
    pub fn font(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.merged_array(id, &format!("font/{}.json", id.path), "providers")
    }
    pub fn language(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("lang/{}.json", id.path))
    }
    pub fn merged_language(&self, id: &ResourceId) -> Result<Map<String, Value>> {
        self.merged_object(id, &format!("lang/{}.json", id.path))
    }
    pub fn sound(&self, id: &ResourceId) -> Result<Option<Vec<u8>>> {
        self.get(id, &format!("sounds/{}.ogg", id.path))
    }
    pub fn item_definition(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("items/{}.json", id.path))
    }
    pub fn particle(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("particles/{}.json", id.path))
    }
    pub fn equipment(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("equipment/{}.json", id.path))
    }
    pub fn post_effect(&self, id: &ResourceId) -> Result<Option<Value>> {
        self.json(id, &format!("post_effect/{}.json", id.path))
    }
    fn merged_array(&self, id: &ResourceId, path: &str, key: &str) -> Result<Option<Value>> {
        let mut list = Vec::new();
        let mut found = false;
        for pack in &self.packs {
            if pack.blocks(id, path) {
                list.clear();
                found = false;
            }
            if let Some(bytes) = pack.read(&format!("assets/{}/{}", id.namespace, path))? {
                let value: Value = serde_json::from_slice(&bytes)?;
                let items = value
                    .get(key)
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow!("{path} must contain {key} array"))?;
                list.extend(items.iter().cloned());
                found = true;
            }
        }
        Ok(found.then(|| {
            let mut object = Map::new();
            object.insert(key.to_string(), Value::Array(list));
            Value::Object(object)
        }))
    }
    fn merged_object(&self, id: &ResourceId, path: &str) -> Result<Map<String, Value>> {
        let mut merged = Map::new();
        for pack in &self.packs {
            if pack.blocks(id, path) {
                merged.clear();
            }
            if let Some(bytes) = pack.read(&format!("assets/{}/{}", id.namespace, path))? {
                let values: Map<String, Value> = serde_json::from_slice(&bytes)?;
                merged.extend(values);
            }
        }
        Ok(merged)
    }
    /// Vanilla sound definitions merge event maps across the stack. A true `replace` resets an event.
    pub fn sound_events(&self, namespace: &str) -> Result<Map<String, Value>> {
        let mut events = Map::new();
        let id = ResourceId::parse(&format!("{namespace}:sounds"))?;
        for pack in &self.packs {
            let path = format!("assets/{namespace}/sounds.json");
            if pack.blocks(&id, "sounds.json") {
                events.clear();
            }
            if let Some(bytes) = pack.read(&path)? {
                let values: Map<String, Value> = serde_json::from_slice(&bytes)?;
                for (name, value) in values {
                    if value.get("replace").and_then(Value::as_bool) == Some(true)
                        || !events.contains_key(&name)
                    {
                        events.insert(name, value);
                    } else if let (Some(existing), Some(next)) = (
                        events
                            .get_mut(&name)
                            .and_then(|v| v.get_mut("sounds"))
                            .and_then(Value::as_array_mut),
                        value.get("sounds").and_then(Value::as_array),
                    ) {
                        existing.extend(next.iter().cloned());
                    }
                }
            }
        }
        Ok(events)
    }
    pub fn list(&self, namespace: &str, prefix: &str) -> Result<Vec<String>> {
        let id = ResourceId::parse(&format!("{namespace}:dummy"))?;
        let mut paths = BTreeMap::new();
        for pack in &self.packs {
            paths.retain(|key: &String, _| {
                !pack.blocks(
                    &id,
                    key.strip_prefix(&format!("assets/{namespace}/"))
                        .unwrap_or(key),
                )
            });
            for path in pack.list(&format!("assets/{namespace}/{prefix}")) {
                paths.insert(path, ());
            }
        }
        Ok(paths.into_keys().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(path: &Path, meta: &str, files: &[(&str, &str)]) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("pack.mcmeta"), meta).unwrap();
        for (name, contents) in files {
            let p = path.join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, contents).unwrap();
        }
    }
    const META: &str = r#"{"pack":{"description":"test","min_format":[97,1],"max_format":[97,1]}}"#;
    #[test]
    fn override_fallback_filter_reload_and_namespace() {
        let tmp = crate::test_dir::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        pack(
            &a,
            META,
            &[
                ("assets/minecraft/textures/block/stone.png", "base"),
                ("assets/other/models/block/x.json", "{}"),
            ],
        );
        pack(
            &b,
            META,
            &[("assets/minecraft/textures/block/stone.png", "override")],
        );
        let mut s = PackStack::open(vec![a.clone(), b.clone()]).unwrap();
        let stone = ResourceId::parse("minecraft:block/stone").unwrap();
        assert_eq!(s.texture(&stone).unwrap().unwrap(), b"override");
        assert!(s
            .model(&ResourceId::parse("other:block/x").unwrap())
            .unwrap()
            .is_some());
        fs::remove_file(b.join("assets/minecraft/textures/block/stone.png")).unwrap();
        assert_eq!(s.texture(&stone).unwrap().unwrap(), b"override"); // snapshot until reload
        s.reload().unwrap();
        assert_eq!(s.texture(&stone).unwrap().unwrap(), b"base");
        fs::write(b.join("pack.mcmeta"), r#"{"pack":{"min_format":[97,1],"max_format":[97,1]},"filter":{"block":[{"namespace":"minecraft","path":"textures/block/stone.png"}]}}"#).unwrap();
        s.reload().unwrap();
        assert!(s.texture(&stone).unwrap().is_none());
        s.set_sources(vec![a.clone()]).unwrap();
        assert_eq!(s.texture(&stone).unwrap().unwrap(), b"base");
        fs::write(b.join("pack.mcmeta"), META).unwrap();
        fs::write(
            b.join("assets/minecraft/textures/block/stone.png"),
            "override2",
        )
        .unwrap();
        s.set_sources(vec![b, a]).unwrap();
        assert_eq!(s.texture(&stone).unwrap().unwrap(), b"base");
    }
    #[test]
    fn overlay_and_malformed() {
        let tmp = crate::test_dir::tempdir().unwrap();
        let p = tmp.path().join("pack");
        pack(
            &p,
            r#"{"pack":{"min_format":97,"max_format":97},"overlays":{"entries":[{"directory":"new","min_format":[97,1],"max_format":[97,1]}]}}"#,
            &[
                ("assets/minecraft/x.txt", "old"),
                ("new/assets/minecraft/x.txt", "new"),
            ],
        );
        let s = PackStack::open(vec![p.clone()]).unwrap();
        assert_eq!(
            s.get(&ResourceId::parse("minecraft:x").unwrap(), "x.txt")
                .unwrap()
                .unwrap(),
            b"new"
        );
        fs::write(p.join("pack.mcmeta"), "{").unwrap();
        assert!(Pack::open(&p).is_err());
        fs::write(
            p.join("pack.mcmeta"),
            r#"{"pack":{"min_format":98,"max_format":98}}"#,
        )
        .unwrap();
        assert!(Pack::open(&p).is_err());
    }
    #[test]
    fn zip_and_traversal() {
        use std::io::Write;
        let tmp = crate::test_dir::tempdir().unwrap();
        let p = tmp.path().join("p.zip");
        let mut z = zip::ZipWriter::new(File::create(&p).unwrap());
        z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(META.as_bytes()).unwrap();
        z.start_file(
            "assets/minecraft/lang/en_us.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        z.write_all(br#"{"x":"y"}"#).unwrap();
        z.finish().unwrap();
        assert!(PackStack::open(vec![p])
            .unwrap()
            .language(&ResourceId::parse("minecraft:en_us").unwrap())
            .unwrap()
            .is_some());
        assert!(validate_path("../evil").is_err());
        let evil = tmp.path().join("evil.zip");
        let mut z = zip::ZipWriter::new(File::create(&evil).unwrap());
        z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(META.as_bytes()).unwrap();
        z.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"bad").unwrap();
        z.finish().unwrap();
        assert!(Pack::open(evil).is_err());
    }
    #[test]
    fn type_specific_merges_and_sound_replace() {
        let tmp = crate::test_dir::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        pack(
            &a,
            META,
            &[
                (
                    "assets/minecraft/atlases/blocks.json",
                    r#"{"sources":[{"type":"single","resource":"a"}]}"#,
                ),
                (
                    "assets/minecraft/font/default.json",
                    r#"{"providers":[{"type":"space","advances":{" ":4}}]}"#,
                ),
                (
                    "assets/minecraft/lang/en_us.json",
                    r#"{"old":"a","same":"a"}"#,
                ),
                (
                    "assets/minecraft/sounds.json",
                    r#"{"event":{"sounds":["a"]}}"#,
                ),
            ],
        );
        pack(
            &b,
            META,
            &[
                (
                    "assets/minecraft/atlases/blocks.json",
                    r#"{"sources":[{"type":"single","resource":"b"}]}"#,
                ),
                (
                    "assets/minecraft/font/default.json",
                    r#"{"providers":[{"type":"space","advances":{"x":2}}]}"#,
                ),
                ("assets/minecraft/lang/en_us.json", r#"{"same":"b"}"#),
                (
                    "assets/minecraft/sounds.json",
                    r#"{"event":{"sounds":["b"]}}"#,
                ),
            ],
        );
        let stack = PackStack::open(vec![a, b]).unwrap();
        assert_eq!(
            stack
                .atlas(&ResourceId::parse("minecraft:blocks").unwrap())
                .unwrap()
                .unwrap()["sources"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            stack
                .font(&ResourceId::parse("minecraft:default").unwrap())
                .unwrap()
                .unwrap()["providers"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let language = stack
            .merged_language(&ResourceId::parse("minecraft:en_us").unwrap())
            .unwrap();
        assert_eq!(language["old"], "a");
        assert_eq!(language["same"], "b");
        assert_eq!(
            stack.sound_events("minecraft").unwrap()["event"]["sounds"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
