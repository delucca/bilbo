use std::collections::HashMap;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::store::{self, Env};

pub const MAGIC: &[u8; 10] = b"BILBOVEC1\n";

#[derive(Debug, Default, PartialEq)]
pub struct Cache {
    /// The model that made every vector; empty for an empty cache.
    pub model: String,
    /// The vector length; 0 when there is no vector.
    pub dims: usize,
    pub vectors: HashMap<u64, Vec<f32>>,
}

impl Cache {
    /// The vector for `key` when `model` made the cache.
    pub fn get(&self, model: &str, key: u64) -> Option<&[f32]> {
        if self.model != model {
            return None;
        }
        self.vectors.get(&key).map(Vec::as_slice)
    }
}

/// FNV-1a 64 of the input's UTF-8 bytes.
pub fn key(input: &str) -> u64 {
    fnv(input.as_bytes())
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `$XDG_CACHE_HOME/bilbo` when that is absolute, else `$HOME/.cache/bilbo`.
pub fn dir(env: &Env) -> Option<PathBuf> {
    if let Some(xdg) = store::absolute(&env.xdg_cache_home) {
        return Some(xdg.join("bilbo"));
    }
    store::absolute(&env.home).map(|home| home.join(".cache/bilbo"))
}

/// `<dir>/<16 lowercase hex digits of FNV-1a 64 of the normalized root>.vectors`.
pub fn path(dir: &Path, root: &Path) -> PathBuf {
    let root: PathBuf = root.components().collect();
    dir.join(format!("{:016x}.vectors", fnv(root.as_os_str().as_bytes())))
}

/// The cache at `path`; missing, unreadable or malformed loads as empty.
pub fn load(path: &Path) -> Cache {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| decode(&bytes))
        .unwrap_or_default()
}

fn decode(bytes: &[u8]) -> Option<Cache> {
    let rest = bytes.strip_prefix(MAGIC.as_slice())?;
    let (model_len, rest) = take_u32(rest)?;
    let model_len = usize::try_from(model_len).ok()?;
    let (model, rest) = rest.split_at_checked(model_len)?;
    let model = std::str::from_utf8(model).ok()?.to_string();
    let (dims, rest) = take_u32(rest)?;
    let (count, rest) = take_u32(rest)?;
    let dims = usize::try_from(dims).ok()?;
    let count = usize::try_from(count).ok()?;
    if count > 0 && dims == 0 {
        return None;
    }
    let record = dims.checked_mul(4)?.checked_add(8)?;
    if rest.len() != count.checked_mul(record)? {
        return None;
    }
    let mut vectors = HashMap::new();
    for chunk in rest.chunks_exact(record) {
        let (id, floats) = chunk.split_at(8);
        let id = u64::from_le_bytes(id.try_into().ok()?);
        let vector = floats
            .chunks_exact(4)
            .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
            .collect();
        vectors.insert(id, vector);
    }
    Some(Cache {
        model,
        dims,
        vectors,
    })
}

fn take_u32(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let (head, rest) = bytes.split_at_checked(4)?;
    Some((u32::from_le_bytes(head.try_into().ok()?), rest))
}

fn encode(cache: &Cache) -> Vec<u8> {
    let mut keys: Vec<u64> = cache.vectors.keys().copied().collect();
    keys.sort_unstable();
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(cache.model.len() as u32).to_le_bytes());
    out.extend_from_slice(cache.model.as_bytes());
    out.extend_from_slice(&(cache.dims as u32).to_le_bytes());
    out.extend_from_slice(&(keys.len() as u32).to_le_bytes());
    for key in keys {
        out.extend_from_slice(&key.to_le_bytes());
        for value in &cache.vectors[&key] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// Writes `<path>.tmp-<pid>`, syncs it and renames it over `path`; creates the folder.
pub fn save(path: &Path, cache: &Cache) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(fail)?;
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = parent.join(name);
    let written = std::fs::File::create(&temp)
        .and_then(|mut file| {
            file.write_all(&encode(cache))?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        fail(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("bilbo-vectors-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cache(order: &[(u64, [f32; 3])]) -> Cache {
        Cache {
            model: "m".into(),
            dims: 3,
            vectors: order.iter().map(|(k, v)| (*k, v.to_vec())).collect(),
        }
    }

    fn two() -> Cache {
        cache(&[(7, [0.5, -1.0, 2.0]), (3, [1.0, 0.0, 0.25])])
    }

    fn env(xdg: Option<&str>, home: Option<&str>) -> Env {
        Env::from_vars(|name| match name {
            "XDG_CACHE_HOME" => xdg.map(Into::into),
            "HOME" => home.map(Into::into),
            _ => None,
        })
    }

    #[test]
    fn fnv_matches_the_published_vectors() {
        assert_eq!(format!("{:016x}", key("")), "cbf29ce484222325");
        assert_eq!(format!("{:016x}", key("a")), "af63dc4c8601ec8c");
        assert_eq!(format!("{:016x}", key("foobar")), "85944171f73967e8");
        let input = "Note store > Layout\nOne flat folder.";
        assert_eq!(format!("{:016x}", key(input)), "8c7dcffc41801d77");
    }

    #[test]
    fn root_key_ignores_trailing_slash() {
        let dir = Path::new("/c");
        assert_eq!(
            path(dir, Path::new("/tmp/a/")),
            path(dir, Path::new("/tmp/a"))
        );
        assert_ne!(
            path(dir, Path::new("/tmp/b")),
            path(dir, Path::new("/tmp/a"))
        );
        let file = path(dir, Path::new("/tmp/a"));
        let name = file.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 16 + ".vectors".len());
        assert!(name.ends_with(".vectors"));
    }

    #[test]
    fn round_trip() {
        let scratch = Scratch::new("round");
        let file = scratch.0.join("c.vectors");
        save(&file, &two()).unwrap();
        assert_eq!(load(&file), two());
    }

    #[test]
    fn bytes_are_deterministic() {
        let scratch = Scratch::new("bytes");
        let a = scratch.0.join("a.vectors");
        let b = scratch.0.join("b.vectors");
        save(&a, &two()).unwrap();
        save(&b, &cache(&[(3, [1.0, 0.0, 0.25]), (7, [0.5, -1.0, 2.0])])).unwrap();
        let bytes = std::fs::read(&a).unwrap();
        assert_eq!(bytes, std::fs::read(&b).unwrap());
        let mut head = b"BILBOVEC1\n".to_vec();
        head.extend_from_slice(&1u32.to_le_bytes());
        head.push(b'm');
        head.extend_from_slice(&3u32.to_le_bytes());
        head.extend_from_slice(&2u32.to_le_bytes());
        head.extend_from_slice(&3u64.to_le_bytes());
        assert!(bytes.starts_with(&head));
        assert_eq!(bytes.len(), head.len() - 8 + 2 * (8 + 12));
    }

    #[test]
    fn truncated_file_loads_empty() {
        let scratch = Scratch::new("truncated");
        let file = scratch.0.join("c.vectors");
        save(&file, &two()).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        for len in 0..bytes.len() {
            std::fs::write(&file, &bytes[..len]).unwrap();
            assert_eq!(load(&file), Cache::default(), "prefix of {len} bytes");
        }
    }

    #[test]
    fn extra_byte_loads_empty() {
        let scratch = Scratch::new("extra");
        let file = scratch.0.join("c.vectors");
        save(&file, &two()).unwrap();
        let mut bytes = std::fs::read(&file).unwrap();
        bytes.push(0);
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(load(&file), Cache::default());
    }

    #[test]
    fn bad_magic_loads_empty() {
        let scratch = Scratch::new("magic");
        let file = scratch.0.join("c.vectors");
        save(&file, &two()).unwrap();
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[0] = b'X';
        std::fs::write(&file, bytes).unwrap();
        assert_eq!(load(&file), Cache::default());
    }

    #[test]
    fn missing_file_loads_empty() {
        let scratch = Scratch::new("missing");
        assert_eq!(load(&scratch.0.join("none.vectors")), Cache::default());
    }

    #[test]
    fn other_model_misses_every_key() {
        let cache = two();
        assert!(cache.get("m", 3).is_some());
        assert!(
            cache
                .vectors
                .keys()
                .all(|k| cache.get("other", *k).is_none())
        );
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let scratch = Scratch::new("temp");
        let file = scratch.0.join("c.vectors");
        save(&file, &two()).unwrap();
        let names: Vec<_> = std::fs::read_dir(&scratch.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["c.vectors"]);
    }

    #[test]
    fn save_creates_the_folder() {
        let scratch = Scratch::new("folder");
        let file = scratch.0.join("deep/er/c.vectors");
        save(&file, &two()).unwrap();
        assert_eq!(load(&file), two());
    }

    #[test]
    fn dir_prefers_absolute_xdg_cache_home() {
        assert_eq!(
            dir(&env(Some("/x"), Some("/h"))),
            Some(PathBuf::from("/x/bilbo"))
        );
        assert_eq!(
            dir(&env(Some("x"), Some("/h"))),
            Some(PathBuf::from("/h/.cache/bilbo"))
        );
        assert_eq!(dir(&env(Some("x"), None)), None);
        assert_eq!(dir(&env(None, Some("h"))), None);
    }
}
