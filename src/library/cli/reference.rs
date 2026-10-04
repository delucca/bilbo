//! Resolving `<corpus>/<name>|<id>[#<anchor>]` to a source or a note.

use std::fs;
use std::path::{Path, PathBuf};

use super::{io_failure, refused, usage};
use crate::citation;
use crate::library::corpus::{self, SourceFile};
use crate::shared::frontmatter;
use crate::shared::markdown::{self, Resolved, Section};
use crate::shared::store::{self, EntryKind};
use crate::{Failure, note};

pub enum Reference {
    Name { corpus: String, name: String },
    Id(String),
}

pub fn parse_reference(reference: &str) -> Result<Reference, Failure> {
    let malformed = || {
        usage(format!(
            "malformed source reference '{reference}': use <corpus>/<name> or a source id"
        ))
    };
    match reference.split_once('/') {
        Some((corpus, name)) if corpus::is_corpus_name(corpus) && corpus::is_source_name(name) => {
            Ok(Reference::Name {
                corpus: corpus.into(),
                name: name.into(),
            })
        }
        None if frontmatter::is_ulid(reference) => Ok(Reference::Id(reference.into())),
        _ => Err(malformed()),
    }
}

/// `<corpus>/<name>` of a source file.
pub fn label(file: &SourceFile) -> String {
    let corpus = file
        .path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{corpus}/{}", file.name)
}

pub fn find(root: &Path, reference: &Reference) -> Result<SourceFile, Failure> {
    let library = store::library_dir(root);
    match reference {
        Reference::Name { corpus, name } => {
            let path = library.join(corpus).join(format!("{name}.md"));
            path.is_file()
                .then(|| corpus::read_source(&path, name.clone()))
                .flatten()
                .ok_or_else(|| {
                    refused(format!(
                        "no source '{corpus}/{name}' in {}",
                        library.display()
                    ))
                })
        }
        Reference::Id(id) => {
            let mut found = Vec::new();
            for (_, dir) in corpus::corpus_dirs(root).map_err(io_failure("read", &library))? {
                let sources = corpus::read_sources(&dir).map_err(io_failure("read", &dir))?;
                found.extend(
                    sources
                        .into_iter()
                        .filter(|s| s.source.id.as_deref() == Some(id)),
                );
            }
            match found.len() {
                0 => Err(refused(note_with_id(root, id).map_or_else(
                    || format!("no source with id {id} in {}", library.display()),
                    |path| {
                        format!(
                            "{id} is the id of the note {}, not of a source",
                            path.display()
                        )
                    },
                ))),
                1 => Ok(found.remove(0)),
                _ => Err(refused(format!(
                    "id {id} belongs to {}",
                    found.iter().map(label).collect::<Vec<_>>().join(" and ")
                ))),
            }
        }
    }
}

/// The source with this id, found through the frontmatter scan, and read whole.
pub fn source_by_id(ids: &citation::Ids, id: &str) -> Result<SourceFile, Failure> {
    let target = ids
        .resolve(id)
        .map_err(|message| refused(format!("{id}: {message}")))?;
    if target.kind != citation::Kind::Source {
        return Err(refused(format!(
            "{id} is the id of {}, not of a source",
            target.path.display()
        )));
    }
    let name = target
        .name
        .rsplit_once('/')
        .map_or(&*target.name, |(_, n)| n);
    corpus::read_source(&target.path, name.to_string())
        .ok_or_else(|| refused(format!("cannot read {}", target.path.display())))
}

fn note_with_id(root: &Path, id: &str) -> Option<PathBuf> {
    store::entries(&root.join("notes"))
        .ok()?
        .into_iter()
        .filter(|e| e.kind == EntryKind::File)
        .find(|e| {
            fs::read(&e.path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .is_some_and(|text| note::read(&text).id.as_deref() == Some(id))
        })
        .map(|e| e.path)
}

/// The reference and the anchor after its `#`, if any.
pub fn split_anchor(reference: &str) -> Result<(&str, Option<&str>), Failure> {
    match reference.split_once('#') {
        Some((_, "")) => Err(usage("the anchor after '#' is empty")),
        Some((reference, anchor)) => Ok((reference, Some(anchor))),
        None => Ok((reference, None)),
    }
}

/// The section the anchor names, or the refusal that lists the sections it matches or says it matches none.
pub fn section_for(sections: &[Section], reference: &str, anchor: &str) -> Result<usize, Failure> {
    match markdown::resolve(sections, anchor) {
        Resolved::One(i) => Ok(i),
        Resolved::Ambiguous(found) => {
            let mut message = format!("'{anchor}' matches several sections of {reference}:");
            for i in found {
                let section = &sections[i];
                message.push_str(&format!(
                    "\n{} (line {})",
                    section.path_text(),
                    section.start
                ));
            }
            Err(refused(message))
        }
        Resolved::Missing => Err(refused(format!("no section '{anchor}' in {reference}"))),
    }
}
