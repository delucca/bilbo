use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::Failure;
use crate::search::vectors::{self, Cache};
use crate::search::{documents, embed, rank};
use crate::shared::{config, store};

const SAVE_EVERY: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(600);

/// The stdout line `embedded <n>, kept <n>, dropped <n>`.
pub fn run(args: &[String], env: &store::Env) -> Result<String, Failure> {
    if let Some(arg) = args.first() {
        return Err(Failure::Usage(if arg.starts_with('-') && arg != "-" {
            format!("unknown option '{arg}'")
        } else {
            format!("unexpected argument '{arg}'")
        }));
    }
    let settings = config::load(env).map_err(Failure::Config)?;
    let Some(embedder) = settings.embedder else {
        let path = settings
            .path
            .map_or("$HOME/.config/bilbo/config".into(), |p| {
                p.display().to_string()
            });
        return Err(Failure::Refused(format!(
            "no embedder configured; set embedder.url in {path}"
        )));
    };
    let root = store::root(env).map_err(Failure::Config)?;
    let notes = root.join("notes");
    if !notes.is_dir() {
        return Err(Failure::Refused(format!("no store at {}", root.display())));
    }
    let dir = store::cache_dir(env).ok_or_else(|| {
        Failure::Config(
            "cannot find the cache folder: set XDG_CACHE_HOME, or HOME, to an absolute path".into(),
        )
    })?;
    let file = vectors::path(&dir, &root);
    let stored = documents::read_notes(&notes)
        .map_err(|e| Failure::Refused(format!("cannot read {}: {e}", notes.display())))?;

    let mut needed: Vec<(u64, String)> = Vec::new();
    let mut seen = HashSet::new();
    for passage in stored.iter().flat_map(|n| &n.document.passages) {
        let Some(input) = rank::input(passage) else {
            continue;
        };
        let key = vectors::key(&input);
        if seen.insert(key) {
            needed.push((key, input));
        }
    }

    let mut cache = vectors::load(&file);
    let dropped = if cache.model != embedder.model {
        let dropped = cache.vectors.len();
        cache = Cache {
            model: embedder.model.clone(),
            ..Cache::default()
        };
        dropped
    } else {
        let before = cache.vectors.len();
        cache.vectors.retain(|key, _| seen.contains(key));
        before - cache.vectors.len()
    };
    if cache.vectors.is_empty() {
        cache.dims = 0;
    }
    let kept = needed
        .iter()
        .filter(|(key, _)| cache.get(&embedder.model, *key).is_some())
        .count();
    let changed = dropped > 0;
    let line = |added: usize| format!("embedded {added}, kept {kept}, dropped {dropped}");

    let missing: Vec<(u64, String)> = needed
        .into_iter()
        .filter(|(key, _)| cache.get(&embedder.model, *key).is_none())
        .collect();
    if missing.is_empty() {
        save_if(changed, &file, &cache).map_err(Failure::Refused)?;
        return Ok(line(0));
    }

    let client = match embed::Client::new(&embedder, |name| std::env::var_os(name), TIMEOUT) {
        Ok(client) => client,
        Err(message) => return Err(refused(message, save_if(changed, &file, &cache))),
    };
    let mut expected = (!cache.vectors.is_empty()).then_some(cache.dims);
    let embed_batch = |batch: &[String]| -> Result<Vec<Vec<f32>>, String> {
        let vectors = client.embed(batch)?;
        let n = vectors.first().map_or(0, Vec::len);
        match expected {
            Some(d) if d != n => {
                return Err(format!(
                    "embedder {} answered vectors of {n} dimensions; the cache holds {d}; delete {} and run bilbo index again",
                    embedder.url,
                    file.display()
                ));
            }
            _ => expected = Some(n),
        }
        Ok(vectors)
    };
    let (added, result) = fill(
        &mut cache,
        &missing,
        embed_batch,
        |c| vectors::save(&file, c),
        Instant::now,
    );
    let saved = save_if(changed || added > 0, &file, &cache);
    match result {
        Err(message) => Err(refused(message, saved)),
        Ok(()) => {
            saved.map_err(Failure::Refused)?;
            Ok(line(added))
        }
    }
}

fn save_if(changed: bool, file: &Path, cache: &Cache) -> Result<(), String> {
    if changed {
        vectors::save(file, cache)
    } else {
        Ok(())
    }
}

/// The failure for `message`, with the error of the save that followed it when that failed differently.
fn refused(message: String, saved: Result<(), String>) -> Failure {
    Failure::Refused(match saved {
        Err(save_error) if save_error != message => format!("{message}\n{save_error}"),
        _ => message,
    })
}

/// Embeds `missing` into `cache` in batches of `embed::BATCH`, calling `save` after each batch that ends 30 s or more
/// after the last save. Returns how many vectors it added, and the error that stopped it.
fn fill(
    cache: &mut Cache,
    missing: &[(u64, String)],
    mut embed: impl FnMut(&[String]) -> Result<Vec<Vec<f32>>, String>,
    mut save: impl FnMut(&Cache) -> Result<(), String>,
    mut now: impl FnMut() -> Instant,
) -> (usize, Result<(), String>) {
    let mut added = 0;
    let mut last = now();
    for chunk in missing.chunks(embed::BATCH) {
        let inputs: Vec<String> = chunk.iter().map(|(_, input)| input.clone()).collect();
        let vectors = match embed(&inputs) {
            Ok(vectors) => vectors,
            Err(message) => return (added, Err(message)),
        };
        for ((key, _), vector) in chunk.iter().zip(vectors) {
            cache.dims = vector.len();
            cache.vectors.insert(*key, vector);
        }
        added += chunk.len();
        let t = now();
        if t.duration_since(last) >= SAVE_EVERY {
            if let Err(message) = save(cache) {
                return (added, Err(message));
            }
            last = t;
        }
    }
    (added, Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(n: usize) -> Vec<(u64, String)> {
        (0..n as u64).map(|k| (k, format!("input {k}"))).collect()
    }

    fn answer(batch: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(vec![vec![1.0, 0.0]; batch.len()])
    }

    #[test]
    fn fill_saves_every_30_seconds() {
        let base = Instant::now();
        let mut calls = 0;
        let now = || {
            calls += 1;
            base + Duration::from_secs(20 * (calls - 1))
        };
        let mut saved = Vec::new();
        let mut cache = Cache::default();
        let (added, result) = fill(
            &mut cache,
            &missing(80),
            answer,
            |c| {
                saved.push(c.vectors.len());
                Ok(())
            },
            now,
        );
        assert_eq!((added, result), (80, Ok(())));
        assert_eq!(saved, [32, 64]);
        assert_eq!(cache.vectors.len(), 80);
        assert_eq!(cache.dims, 2);
    }

    #[test]
    fn fill_keeps_vectors_before_a_failure() {
        let mut calls = 0;
        let mut cache = Cache::default();
        let (added, result) = fill(
            &mut cache,
            &missing(80),
            |batch| {
                calls += 1;
                if calls == 3 {
                    Err("down".into())
                } else {
                    answer(batch)
                }
            },
            |_| Ok(()),
            Instant::now,
        );
        assert_eq!((added, result), (32, Err("down".to_string())));
        assert_eq!(cache.vectors.len(), 32);
    }

    #[test]
    fn a_repeated_save_error_prints_once() {
        let text = |f: Failure| match f {
            Failure::Refused(m) => m,
            _ => unreachable!(),
        };
        let same = refused(
            "cannot write x: no".into(),
            Err("cannot write x: no".into()),
        );
        assert_eq!(text(same), "cannot write x: no");
        let other = refused("down".into(), Err("cannot write x: no".into()));
        assert_eq!(text(other), "down\ncannot write x: no");
        assert_eq!(text(refused("down".into(), Ok(()))), "down");
    }

    #[test]
    fn fill_sends_batches_of_16() {
        let mut sizes = Vec::new();
        let mut cache = Cache::default();
        let (added, result) = fill(
            &mut cache,
            &missing(40),
            |batch| {
                sizes.push(batch.len());
                answer(batch)
            },
            |_| Ok(()),
            Instant::now,
        );
        assert_eq!((added, result), (40, Ok(())));
        assert_eq!(sizes, [16, 16, 8]);
    }
}
