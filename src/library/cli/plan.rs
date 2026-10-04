//! `bilbo library plan`.

use super::reference::{find, label, parse_reference, section_for, split_anchor};
use super::{Args, Output, grouped, io_failure, refused, root, sections, state_failure, usage};
use crate::Failure;
use crate::library::corpus::SourceFile;
use crate::library::reading;
use crate::library::source;
use crate::shared::frontmatter;
use crate::shared::markdown::{self, Section};
use crate::shared::store;

/// A whole number of `min` or more, and of `max` or less when there is a limit.
fn whole(name: &str, value: &str, min: usize, max: Option<usize>) -> Result<usize, Failure> {
    let rule = match max {
        Some(max) => format!("from {} to {}", grouped(min), grouped(max)),
        None => format!("of {} or more", grouped(min)),
    };
    value
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| value.parse::<usize>().ok())
        .flatten()
        .filter(|n| *n >= min && max.is_none_or(|max| *n <= max))
        .ok_or_else(|| usage(format!("{name} '{value}' is not a whole number {rule}")))
}

fn plan_options(args: &Args) -> Result<reading::Options, Failure> {
    let mut options = reading::Options::default();
    if let Some(value) = args.one("--budget-tokens") {
        options.budget_tokens = whole("--budget-tokens", value, reading::MIN_BUDGET_TOKENS, None)?;
    }
    if let Some(value) = args.one("--slice-bytes") {
        options.slice_bytes = whole(
            "--slice-bytes",
            value,
            reading::MIN_SLICE_BYTES,
            Some(reading::MAX_SLICE_BYTES),
        )?;
    }
    if let Some(value) = args.one("--slice-lines") {
        options.slice_lines = Some(whole(
            "--slice-lines",
            value,
            reading::MIN_SLICE_LINES,
            None,
        )?);
    }
    Ok(options)
}

pub fn run(args: &Args, env: &store::Env) -> Result<Output, Failure> {
    let references = &args.positional[1..];
    if references.is_empty() {
        return Err(usage("missing <ref>"));
    }
    let options = plan_options(args)?;
    let root = root(env)?;
    let plans = store::plans_dir(env).ok_or_else(state_failure)?;

    let mut files: Vec<SourceFile> = Vec::new();
    let mut picks: Vec<reading::Pick> = Vec::new();
    for reference in references {
        let (name, anchor) = split_anchor(reference)?;
        let file = find(&root, &parse_reference(name)?)?;
        let label = label(&file);
        let lines = markdown::lines(&file.text);
        let sections = sections(&file);
        let (Some(id), Some(digest)) = (&file.source.id, &file.source.digest) else {
            return Err(refused(format!(
                "{label} has no valid id or digest; run bilbo check"
            )));
        };
        let (start, end) = match anchor {
            Some(anchor) => {
                let section = &sections[section_for(&sections, name, anchor)?];
                (section.start, section.end)
            }
            None if source::is_catalog(file.body().len(), &sections) => {
                return Err(refused(format!(
                    "{label} is a catalog, too big to read whole: pick a section as '{label}#<anchor>'; bilbo library show {label} lists them"
                )));
            }
            None => (
                file.source.body_start,
                lines.len().max(file.source.body_start),
            ),
        };
        picks.push(reading::Pick {
            reference: reference.clone(),
            id: id.clone(),
            corpus: label.split('/').next().unwrap_or_default().into(),
            name: file.name.clone(),
            start,
            end,
            digest: digest.clone(),
            body_start: file.source.body_start,
        });
        files.push(file);
    }
    if let Some((first, second)) = reading::overlap(&picks) {
        return Err(usage(format!(
            "'{}' and '{}' overlap: pick each line once",
            picks[first].reference, picks[second].reference
        )));
    }

    let lines: Vec<Vec<&str>> = files.iter().map(|f| markdown::lines(&f.text)).collect();
    let outlines: Vec<Vec<Section>> = files.iter().map(sections).collect();
    let materials: Vec<reading::Material> = lines
        .iter()
        .zip(&outlines)
        .map(|(lines, sections)| reading::Material { lines, sections })
        .collect();
    let id =
        frontmatter::mint_ulid().map_err(|e| refused(format!("cannot read /dev/urandom: {e}")))?;
    let plan = reading::build(
        id,
        root.display().to_string(),
        jiff::Timestamp::now().to_string(),
        options,
        picks,
        &materials,
    );

    reading::prune(&plans, std::time::SystemTime::now());
    reading::save(&plans, &plan).map_err(io_failure("write", &plans))?;

    let partitions = plan.partitions();
    let mut out = vec![
        format!("plan: {}", plan.id),
        format!("picks: {}", plan.picks.len()),
        format!("slices: {}", plan.slices.len()),
        format!("tokens: {}", plan.tokens()),
        format!("partitions: {}", partitions.len()),
    ];
    out.extend(partitions.iter().map(|p| {
        format!(
            "partition {}: slices {}-{}, {} tokens",
            p.number, p.first, p.last, p.tokens
        )
    }));
    out.push(String::new());
    for (i, slice) in plan.slices.iter().enumerate() {
        let pick = &plan.picks[slice.pick];
        let path = reading::heading_path(&outlines[slice.pick], slice.start);
        out.push(format!(
            "{}\t{}\t{}\t{}-{}\t{} tokens\t{}",
            i + 1,
            slice.partition,
            pick.label(),
            slice.start,
            slice.end,
            slice.tokens,
            path.as_deref().unwrap_or("-")
        ));
    }
    Ok(Output::lines(out))
}
