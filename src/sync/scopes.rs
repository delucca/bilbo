//! The owner's scopes on a transport: their chains verified, their names opened, and the one picked by name.

use std::collections::BTreeSet;

use crate::identity::manifest::{self, Recipient, Scope};
use crate::sync::transport::{self, Transport};

/// A scope of the owner that the reader opened, with its verified chain.
pub struct Found {
    pub id: String,
    pub name: String,
    /// The devices its latest version lists.
    pub devices: usize,
    pub scope: Scope,
}

/// The owner's scopes on a transport, as one reader sees them.
#[derive(Default)]
pub struct Listing {
    /// The scopes the reader opens.
    pub found: Vec<Found>,
    /// Every scope folder on the transport, of any owner.
    pub ids: BTreeSet<String>,
    /// The owner's scopes the reader does not open: it is not listed, or the chain does not verify for it.
    pub closed: Vec<String>,
    /// The scopes whose chain does not verify, with the reason, whoever signed them.
    pub broken: Vec<(String, String)>,
    /// The scopes whose version 1 is missing or does not verify, so nobody can say whose they are: one may be still
    /// arriving, and a version 1 written beside it would fork it.
    pub unattributed: Vec<(String, String)>,
}

impl Listing {
    /// Whether the transport holds a scope of the owner that the reader cannot open, and the reader is in none there:
    /// a version 1 written now would hide that scope.
    pub fn outsider(&self) -> bool {
        self.found.is_empty() && !self.closed.is_empty()
    }
}

/// The versions of scope `id` as the transport holds them: the valid prefix, and why the next one is not valid, if it
/// is not. A scope with no manifest has neither.
pub fn chain(t: &dyn Transport, id: &str) -> Result<Scope, String> {
    let Some(highest) = t.highest_manifest(id)? else {
        return Ok(manifest::verify_scope(id, &[]));
    };
    let mut files = Vec::new();
    let mut unreadable = None;
    for n in 1..=highest {
        match t.get(&transport::manifest_path(id, n)) {
            Ok(Some(bytes)) => files.push(bytes),
            Ok(None) => break,
            Err(why) => {
                unreadable = Some(why);
                break;
            }
        }
        if manifest::verify_scope(id, &files).invalid.is_some() {
            break;
        }
    }
    let mut scope = manifest::verify_scope(id, &files);
    if let (None, Some(why)) = (&scope.invalid, unreadable) {
        scope.invalid = Some(manifest::Invalid {
            n: files.len() as u64 + 1,
            why,
        });
    } else if scope.invalid.is_none() && (files.len() as u64) < highest {
        scope.invalid = Some(manifest::Invalid {
            n: files.len() as u64 + 1,
            why: "it is missing, and a later version is there".into(),
        });
    }
    Ok(scope)
}

/// The scopes of `owner` on `t`, each chain verified and each name opened by `who`. A scope of another owner is
/// left out.
pub fn list(t: &dyn Transport, owner: &[u8; 32], who: &Recipient) -> Result<Listing, String> {
    let mut listing = Listing {
        found: Vec::new(),
        ids: BTreeSet::new(),
        closed: Vec::new(),
        broken: Vec::new(),
        unattributed: Vec::new(),
    };
    for id in t.scopes()? {
        listing.ids.insert(id.clone());
        let scope = chain(t, &id)?;
        if scope.versions.is_empty() && scope.invalid.is_none() {
            continue;
        }
        match scope.owner() {
            Some(signer) if signer != *owner => continue,
            Some(_) => {}
            None => {
                if let Some(invalid) = &scope.invalid {
                    let entry = (id, invalid.to_string());
                    listing.broken.push(entry.clone());
                    listing.unattributed.push(entry);
                }
                continue;
            }
        }
        if let Some(invalid) = &scope.invalid {
            listing.broken.push((id.clone(), invalid.to_string()));
            listing.closed.push(id);
            continue;
        }
        match manifest::open(&scope, who) {
            Ok(Some(opened)) => {
                let devices = scope.latest().map_or(0, |v| v.manifest.devices.len());
                listing.found.push(Found {
                    id,
                    name: opened.name,
                    devices,
                    scope,
                });
            }
            Ok(None) => listing.closed.push(id),
            Err(invalid) => {
                listing.broken.push((id.clone(), invalid.to_string()));
                listing.closed.push(id);
            }
        }
    }
    Ok(listing)
}

/// The scope picked for a name, and the other scopes with that name.
pub struct Pick<'a> {
    pub chosen: &'a Found,
    pub rivals: Vec<&'a Found>,
}

/// The scope named `name` that lists more devices, on a tie the lower id.
pub fn pick<'a>(found: &'a [Found], name: &str) -> Option<Pick<'a>> {
    let mut same: Vec<&Found> = found.iter().filter(|f| f.name == name).collect();
    same.sort_by(|a, b| b.devices.cmp(&a.devices).then_with(|| a.id.cmp(&b.id)));
    let (chosen, rivals) = same.split_first()?;
    Some(Pick {
        chosen,
        rivals: rivals.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keys::{Device, Identity, Owner};
    use crate::identity::manifest::Member;
    use crate::sync::transport::{Folder, Put};
    use std::fs;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bilbo-scopes-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("store")).unwrap();
        fs::create_dir_all(dir.join("folder")).unwrap();
        Scratch(dir)
    }

    fn identity(owner: u8, name: &str, seed: u8) -> Identity {
        Identity {
            owner: Owner::derive(&[owner; 16]).file(),
            device: Device::from_seeds(name, &[seed; 32], &[seed + 1; 32]),
        }
    }

    /// Creates scope `name` by `who` with `more` listed, and puts every version in the folder.
    fn publish(d: &Scratch, who: &Identity, name: &str, more: &[&Identity]) -> String {
        let store = d.0.join("store");
        let lock = manifest::lock(&store).unwrap();
        let members: Vec<Member> = more.iter().map(|i| Member::of(&i.device)).collect();
        let sid = manifest::create(&lock, who, name, "file://", &members)
            .unwrap()
            .scope;
        let t = Folder::new(d.0.join("folder"), &who.device.id());
        for v in &manifest::read_scope(&store, &sid).unwrap().versions {
            let path = transport::manifest_path(&sid, v.manifest.n);
            assert_eq!(t.create(&path, &v.bytes), Put::Created);
        }
        sid
    }

    fn folder(d: &Scratch) -> Folder {
        Folder::new(d.0.join("folder"), &identity(0, "x", 1).device.id())
    }

    fn manifest_file(d: &Scratch, sid: &str, n: u64) -> PathBuf {
        d.0.join("folder").join(transport::manifest_path(sid, n))
    }

    fn as_device(d: &Scratch, who: &Identity) -> Listing {
        let me = Recipient::device(&who.device);
        list(&folder(d), &who.owner.sign.public(), &me).unwrap()
    }

    #[test]
    fn a_device_opens_the_scopes_that_list_it_and_the_phrase_opens_all() {
        let d = scratch("opens");
        let (a, b, c) = (
            identity(0, "a", 1),
            identity(0, "b", 3),
            identity(0, "c", 5),
        );
        let personal = publish(&d, &a, "personal", &[&b]);
        let shared = publish(&d, &a, "shared", &[]);
        let listing = as_device(&d, &b);
        assert_eq!(listing.found.len(), 1);
        assert_eq!(
            (listing.found[0].id.as_str(), listing.found[0].name.as_str()),
            (personal.as_str(), "personal")
        );
        assert_eq!(listing.found[0].devices, 2);
        assert_eq!(listing.closed, std::slice::from_ref(&shared));
        assert!(!listing.outsider());
        let owner = Owner::derive(&[0; 16]);
        let all = list(
            &folder(&d),
            &owner.sign.public(),
            &Recipient::Owner(&owner.box_secret),
        )
        .unwrap();
        let mut names: Vec<&str> = all.found.iter().map(|f| f.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["personal", "shared"]);
        assert!(all.closed.is_empty() && all.broken.is_empty());
        assert!(as_device(&d, &c).outsider());
    }

    #[test]
    fn an_empty_folder_or_one_of_another_owner_makes_no_outsider() {
        let d = scratch("others");
        let me = identity(0, "a", 1);
        assert!(!as_device(&d, &me).outsider());
        let gandalf = identity(1, "gandalf", 5);
        let grey = publish(&d, &gandalf, "grey", &[]);
        let listing = as_device(&d, &me);
        assert!(listing.ids.contains(&grey));
        assert!(listing.found.is_empty() && listing.closed.is_empty() && listing.broken.is_empty());
        assert!(!listing.outsider());
        assert_eq!(listing.ids.len(), 1);
    }

    #[test]
    fn a_scope_with_no_valid_version_1_is_broken_and_unattributed() {
        let d = scratch("broken");
        let a = identity(0, "a", 1);
        let good = publish(&d, &a, "good", &[]);
        let bad = publish(&d, &a, "bad", &[]);
        let path = manifest_file(&d, &bad, 1);
        let mut bytes = fs::read(&path).unwrap();
        let at = bytes.len() / 2;
        bytes[at] ^= 1;
        fs::write(&path, bytes).unwrap();
        let listing = as_device(&d, &a);
        assert_eq!(listing.found.len(), 1);
        assert_eq!(listing.found[0].id, good);
        assert_eq!(listing.broken.len(), 1);
        assert_eq!(listing.broken[0].0, bad);
        assert!(listing.broken[0].1.contains("manifest/1.json"));
        assert!(listing.closed.is_empty(), "no owner can be told for it");
        assert_eq!(listing.unattributed, listing.broken);
    }

    #[test]
    fn a_later_version_that_fails_closes_a_scope_whose_owner_is_known() {
        let d = scratch("later");
        let a = identity(0, "a", 1);
        let sid = publish(&d, &a, "personal", &[]);
        fs::write(manifest_file(&d, &sid, 2), b"not a manifest").unwrap();
        let listing = as_device(&d, &a);
        assert!(listing.found.is_empty());
        assert_eq!(listing.closed, std::slice::from_ref(&sid));
        assert_eq!(listing.broken[0].0, sid);
        assert!(listing.outsider());
    }

    #[test]
    fn a_missing_version_below_a_later_one_is_broken() {
        let d = scratch("gap");
        let a = identity(0, "a", 1);
        let sid = publish(&d, &a, "personal", &[]);
        fs::write(manifest_file(&d, &sid, 3), b"x").unwrap();
        let listing = as_device(&d, &a);
        assert_eq!(listing.closed, [sid]);
        assert!(listing.broken[0].1.contains("it is missing"));
    }

    #[test]
    fn a_folder_that_cannot_be_read_is_an_error() {
        let d = scratch("unreadable");
        let t = Folder::new(d.0.join("usb"), "x");
        let a = identity(0, "a", 1);
        let err = list(&t, &a.owner.sign.public(), &Recipient::device(&a.device))
            .err()
            .unwrap();
        assert_eq!(err, "the folder does not exist");
    }

    #[test]
    fn the_scope_listing_more_devices_wins_and_a_tie_takes_the_lower_id() {
        let d = scratch("pick");
        let a = identity(0, "a", 1);
        let (b, c) = (identity(0, "b", 3), identity(0, "c", 5));
        let big = publish(&d, &a, "personal", &[&b, &c]);
        let small = publish(&d, &a, "personal", &[]);
        publish(&d, &a, "shared", &[]);
        let owner = Owner::derive(&[0; 16]);
        let all = list(
            &folder(&d),
            &owner.sign.public(),
            &Recipient::Owner(&owner.box_secret),
        )
        .unwrap();
        let picked = pick(&all.found, "personal").unwrap();
        assert_eq!(picked.chosen.id, big);
        assert_eq!(picked.rivals.len(), 1);
        assert_eq!(picked.rivals[0].id, small);
        assert!(pick(&all.found, "nothing").is_none());
        let twin = publish(&d, &a, "twin", &[]);
        let twin2 = publish(&d, &a, "twin", &[]);
        let all = list(
            &folder(&d),
            &owner.sign.public(),
            &Recipient::Owner(&owner.box_secret),
        )
        .unwrap();
        let picked = pick(&all.found, "twin").unwrap();
        assert_eq!(picked.chosen.id, twin.min(twin2));
    }

    #[test]
    fn a_name_that_is_not_opened_cannot_be_picked() {
        let d = scratch("closed_name");
        let (a, b) = (identity(0, "a", 1), identity(0, "b", 3));
        publish(&d, &a, "personal", &[]);
        let listing = as_device(&d, &b);
        assert!(pick(&listing.found, "personal").is_none());
    }

    #[test]
    fn a_scope_whose_version_1_is_still_arriving_is_unattributed() {
        let d = scratch("arriving");
        let a = identity(0, "a", 1);
        let sid = publish(&d, &a, "personal", &[]);
        let v2 = d.0.join("folder").join(transport::manifest_path(&sid, 2));
        fs::write(&v2, fs::read(manifest_file(&d, &sid, 1)).unwrap()).unwrap();
        fs::remove_file(manifest_file(&d, &sid, 1)).unwrap();
        let listing = as_device(&d, &a);
        assert!(listing.found.is_empty() && listing.closed.is_empty());
        assert_eq!(listing.unattributed.len(), 1);
        assert_eq!(listing.unattributed[0].0, sid);
        assert!(listing.unattributed[0].1.contains("manifest/1.json"));
        assert!(!listing.outsider());
    }

    #[test]
    fn reading_stops_at_the_first_version_that_fails() {
        let d = scratch("stops");
        let a = identity(0, "a", 1);
        let sid = publish(&d, &a, "personal", &[]);
        fs::write(manifest_file(&d, &sid, 2), b"junk").unwrap();
        for n in 3..=40 {
            fs::write(manifest_file(&d, &sid, n), vec![0u8; 1024]).unwrap();
        }
        let listing = as_device(&d, &a);
        assert!(listing.broken[0].1.starts_with("manifest/2.json"));
    }

    #[test]
    fn a_manifest_that_cannot_be_read_ends_the_chain_there() {
        let d = scratch("too-big");
        let a = identity(0, "a", 1);
        let sid = publish(&d, &a, "personal", &[]);
        let file = fs::File::create(manifest_file(&d, &sid, 2)).unwrap();
        file.set_len(transport::OBJECT_MAX + 1).unwrap();
        let scope = chain(&folder(&d), &sid).unwrap();
        assert_eq!(scope.versions.len(), 1);
        assert_eq!(scope.invalid.map(|i| i.n), Some(2));
    }
}
