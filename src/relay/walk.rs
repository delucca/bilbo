//! The relay's start-up walk: every scope in the data folder read as a `file://` transport, its chain verified as a
//! device verifies it, its seqs and owner checked, and its state rebuilt. Nothing on disk is trusted or changed.

use std::collections::BTreeMap;

use super::admit::{Held, Standing};
use super::store::Data;
use crate::identity::keys;
use crate::identity::manifest::Manifest;
use crate::sync::scopes;
use crate::sync::transport::{self, Folder, Transport};

/// The scopes under `<data>/scopes/`, each `Valid`, `NotAdmitted` or `Invalid`, with its byte total, latest version
/// and each device's highest seq. Each scope that is not valid gets one line on `log`.
pub fn walk(
    data: &Data,
    owners: &[String],
    log: &(dyn Fn(&str) + Sync),
) -> Result<BTreeMap<String, Held>, String> {
    let folder = Folder::new(data.root().to_path_buf(), "");
    let mut held = BTreeMap::new();
    for id in folder.scopes()? {
        let chain = scopes::chain(&folder, &id)?;
        let mut bytes = 0;
        let mut highest = BTreeMap::new();
        let mut gap = None;
        for device in folder.devices(&id)? {
            let seqs = folder.list_after(&id, &device, 0)?;
            for (seq, expected) in seqs.iter().zip(1u64..) {
                if *seq != expected && gap.is_none() {
                    gap = Some(format!("device {device} has no segment {expected}"));
                }
                bytes += size(data, &transport::segment_path(&id, &device, *seq));
            }
            if let Some(last) = seqs.last() {
                highest.insert(device, *last);
            }
        }
        let manifests = folder.highest_manifest(&id)?;
        for n in 1..=manifests.unwrap_or(0) {
            bytes += size(data, &transport::manifest_path(&id, n));
        }
        if manifests.is_none() && highest.is_empty() {
            continue;
        }
        let owner = chain
            .owner()
            .or_else(|| first_owner(&folder, &id))
            .map(|key| keys::owner_fingerprint(&key));
        let standing = if let Some(owner) = owner.filter(|o| !owners.contains(o)) {
            log(&format!(
                "scope {id} is not admitted: its owner {owner} was not passed with --owner"
            ));
            Standing::NotAdmitted
        } else if let Some(invalid) = &chain.invalid {
            Standing::Invalid(format!("manifest/{}.json: {}", invalid.n, invalid.why))
        } else if let Some(gap) = gap {
            Standing::Invalid(gap)
        } else if chain.versions.is_empty() {
            Standing::Invalid("it has no manifest".into())
        } else {
            Standing::Valid
        };
        if let Standing::Invalid(why) = &standing {
            log(&format!("scope {id} is invalid: {why}"));
        }
        held.insert(
            id,
            Held {
                chain,
                standing,
                bytes,
                reserved: 0,
                highest,
            },
        );
    }
    Ok(held)
}

/// The owner version 1 names, read without verifying it, for a scope whose chain failed at version 1.
fn first_owner(folder: &Folder, id: &str) -> Option<[u8; 32]> {
    let bytes = folder.get(&transport::manifest_path(id, 1)).ok()??;
    let manifest: Manifest = serde_json::from_slice(&bytes).ok()?;
    keys::unhex(&manifest.owner)
}

/// The size of the object at `path`, 0 for anything that is not a regular file (a symbolic link is not followed).
fn size(data: &Data, path: &str) -> u64 {
    data.root()
        .join(path)
        .symlink_metadata()
        .ok()
        .filter(|meta| meta.is_file())
        .map_or(0, |meta| meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keys::{Device, Identity, Owner};
    use crate::identity::manifest;
    use crate::shared::hash;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-walk-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("store")).unwrap();
        fs::create_dir_all(dir.join("data")).unwrap();
        Scratch(dir)
    }

    fn identity(owner: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[owner; 16]).file(),
            device: Device::from_seeds("a", &[1; 32], &[2; 32]),
        }
    }

    fn print(who: &Identity) -> String {
        keys::owner_fingerprint(&who.owner.sign.public())
    }

    fn put(root: &Path, path: &str, bytes: &[u8]) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }

    /// A scope of `who` with `versions` versions in `root`: each next one changes only the transport pin.
    fn scope(d: &Scratch, who: &Identity, versions: u64) -> String {
        let lock = manifest::lock(&d.0.join("store")).unwrap();
        let sid = manifest::create(&lock, who, "personal", "file://", &[])
            .unwrap()
            .scope;
        let root = d.0.join("data");
        let mut files = manifest::read_scope(&d.0.join("store"), &sid)
            .unwrap()
            .versions
            .remove(0)
            .bytes;
        put(&root, &transport::manifest_path(&sid, 1), &files);
        for n in 2..=versions {
            let mut m: Manifest = serde_json::from_slice(&files).unwrap();
            m.n = n;
            m.prev = Some(hash::sha256_hex(&files));
            m.transport = format!("https://relay.example/{n}");
            m.sig.clear();
            files = manifest::signed(m, &who.owner.sign).1;
            put(&root, &transport::manifest_path(&sid, n), &files);
        }
        sid
    }

    fn segment(d: &Scratch, sid: &str, device: &str, seq: u64, bytes: &[u8]) {
        put(
            &d.0.join("data"),
            &transport::segment_path(sid, device, seq),
            bytes,
        );
    }

    const DEVICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaab";

    fn run(d: &Scratch, owners: &[String]) -> (BTreeMap<String, Held>, Vec<String>) {
        let data = Data::open(&d.0.join("data")).unwrap();
        let lines = Mutex::new(Vec::new());
        let held = walk(&data, owners, &|line| {
            lines.lock().unwrap().push(line.to_string())
        })
        .unwrap();
        (held, lines.into_inner().unwrap())
    }

    #[test]
    fn a_valid_scope_is_held_with_its_state_and_logs_nothing() {
        let d = scratch("valid");
        let who = identity(0);
        let sid = scope(&d, &who, 2);
        segment(&d, &sid, DEVICE, 1, b"one");
        segment(&d, &sid, DEVICE, 2, b"three");
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbc";
        segment(&d, &sid, other, 1, b"xy");
        let (held, lines) = run(&d, &[print(&who)]);
        assert!(lines.is_empty(), "{lines:?}");
        let h = &held[&sid];
        assert_eq!(h.standing, Standing::Valid);
        assert_eq!(h.chain.latest().unwrap().manifest.n, 2);
        assert_eq!(h.highest[DEVICE], 2);
        assert_eq!(h.highest[other], 1);
        assert_eq!(h.reserved, 0);
        let manifests: u64 = (1..=2)
            .map(|n| {
                fs::metadata(d.0.join("data").join(transport::manifest_path(&sid, n)))
                    .unwrap()
                    .len()
            })
            .sum();
        assert_eq!(h.bytes, manifests + 3 + 5 + 2);
    }

    #[test]
    fn two_owners_are_both_admitted() {
        let d = scratch("two");
        let (a, b) = (identity(0), identity(7));
        let (sa, sb) = (scope(&d, &a, 1), scope(&d, &b, 1));
        let (held, lines) = run(&d, &[print(&a), print(&b)]);
        assert!(lines.is_empty());
        assert_eq!(held[&sa].standing, Standing::Valid);
        assert_eq!(held[&sb].standing, Standing::Valid);
    }

    #[test]
    fn a_hand_edited_manifest_marks_the_scope_invalid_once_and_changes_nothing() {
        let d = scratch("edited");
        let who = identity(0);
        let sid = scope(&d, &who, 2);
        let path = d.0.join("data").join(transport::manifest_path(&sid, 2));
        let mut text = fs::read_to_string(&path).unwrap();
        text = text.replacen("relay.example", "relay.exampla", 1);
        fs::write(&path, &text).unwrap();
        let (held, lines) = run(&d, &[print(&who)]);
        let h = &held[&sid];
        assert!(
            matches!(&h.standing, Standing::Invalid(why) if why.starts_with("manifest/2.json: "))
        );
        assert_eq!(h.chain.versions.len(), 1);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with(&format!("scope {sid} is invalid: manifest/2.json: ")));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn a_seq_gap_marks_the_scope_invalid() {
        let d = scratch("gap");
        let who = identity(0);
        let sid = scope(&d, &who, 1);
        segment(&d, &sid, DEVICE, 1, b"a");
        segment(&d, &sid, DEVICE, 3, b"c");
        let (held, lines) = run(&d, &[print(&who)]);
        let why = format!("device {DEVICE} has no segment 2");
        assert_eq!(held[&sid].standing, Standing::Invalid(why.clone()));
        assert_eq!(lines, vec![format!("scope {sid} is invalid: {why}")]);
    }

    #[test]
    fn seqs_that_do_not_start_at_one_are_a_gap() {
        let d = scratch("start");
        let who = identity(0);
        let sid = scope(&d, &who, 1);
        segment(&d, &sid, DEVICE, 2, b"b");
        let (held, _) = run(&d, &[print(&who)]);
        assert!(matches!(held[&sid].standing, Standing::Invalid(_)));
    }

    #[test]
    fn an_owner_not_passed_is_not_admitted_and_still_held() {
        let d = scratch("owner");
        let (a, b) = (identity(0), identity(7));
        let (sa, sb) = (scope(&d, &a, 1), scope(&d, &b, 1));
        let (held, lines) = run(&d, &[print(&a)]);
        assert_eq!(held[&sa].standing, Standing::Valid);
        assert_eq!(held[&sb].standing, Standing::NotAdmitted);
        assert_eq!(held[&sb].chain.versions.len(), 1);
        assert_eq!(
            lines,
            vec![format!(
                "scope {sb} is not admitted: its owner {} was not passed with --owner",
                print(&b)
            )]
        );
    }

    #[test]
    fn a_broken_scope_of_an_owner_not_passed_is_not_admitted() {
        let d = scratch("broken-unadmitted");
        let (a, b) = (identity(0), identity(7));
        let sid = scope(&d, &b, 2);
        let path = d.0.join("data").join(transport::manifest_path(&sid, 2));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replacen("relay.example", "relay.exampla", 1)).unwrap();
        let first = d.0.join("data").join(transport::manifest_path(&sid, 1));
        let (held, lines) = run(&d, &[print(&a)]);
        assert_eq!(held[&sid].standing, Standing::NotAdmitted);
        assert_eq!(
            lines,
            vec![format!(
                "scope {sid} is not admitted: its owner {} was not passed with --owner",
                print(&b)
            )]
        );
        let mut bad = fs::read_to_string(&first).unwrap();
        bad = bad.replacen("\"sig\":\"", "\"sig\":\"00", 1);
        fs::write(&first, bad).unwrap();
        let (held, _) = run(&d, &[print(&a)]);
        assert_eq!(held[&sid].standing, Standing::NotAdmitted);
        let (held, _) = run(&d, &[print(&b)]);
        assert!(matches!(held[&sid].standing, Standing::Invalid(_)));
    }

    #[test]
    fn a_copied_file_folder_is_served_as_if_created_through_the_relay() {
        let d = scratch("copied");
        let who = identity(0);
        let sid = scope(&d, &who, 2);
        segment(&d, &sid, DEVICE, 1, b"x");
        let (held, lines) = run(&d, &[print(&who)]);
        assert!(lines.is_empty());
        assert_eq!(held[&sid].standing, Standing::Valid);
        assert_eq!(held[&sid].highest[DEVICE], 1);
    }

    #[test]
    fn files_outside_the_grammar_are_ignored() {
        let d = scratch("grammar");
        let who = identity(0);
        let sid = scope(&d, &who, 1);
        segment(&d, &sid, DEVICE, 1, b"x");
        let root = d.0.join("data");
        put(
            &root,
            &format!("scopes/{sid}/devices/{DEVICE}/notes.txt"),
            b"zz",
        );
        put(
            &root,
            &format!("scopes/{sid}/devices/{DEVICE}/7.seg"),
            b"zz",
        );
        put(&root, &format!("scopes/{sid}/manifest/02.json"), b"zz");
        put(&root, &format!("scopes/{sid}/stray"), b"zz");
        put(&root, "scopes/not-a-scope/manifest/1.json", b"zz");
        put(&root, "pair/abc/1.msg", b"zz");
        fs::create_dir_all(root.join(format!("scopes/{sid}/devices/not-a-device"))).unwrap();
        let (held, lines) = run(&d, &[print(&who)]);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(held.keys().collect::<Vec<_>>(), vec![&sid]);
        assert_eq!(held[&sid].standing, Standing::Valid);
        assert_eq!(held[&sid].highest.len(), 1);
    }

    #[test]
    fn an_empty_data_folder_holds_nothing() {
        let d = scratch("empty");
        let (held, lines) = run(&d, &[print(&identity(0))]);
        assert!(held.is_empty() && lines.is_empty());
    }
}
