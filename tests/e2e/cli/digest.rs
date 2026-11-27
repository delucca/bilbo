use std::fs;
use std::process::{Command, Stdio};

use super::binary;
use super::scratch::{Scratch, closed, remembered};

const OTHER: &str = "11111111-2222-4333-8444-555555555555";
const PROMPT: &str = "garden beds carrots";
const OFF: &str = "digest.markdown.floor = 0\ndigest.session.floor = 0\ndigest.note.floor = 0\ndigest.session.old_after = \"36500d\"\ndigest.session.subagents = false\ndigest.session.parts = false\ndigest.note.width = 300\ndigest.summary.enabled = true\n";

fn claude(session: &str, prompt: &str) -> Vec<u8> {
    format!(
        "{{\"session_id\":\"{session}\",\"transcript_path\":\"/h/t.jsonl\",\"cwd\":\"/h\",\"permission_mode\":\"default\",\"hook_event_name\":\"UserPromptSubmit\",\"prompt\":\"{prompt}\"}}"
    )
    .into_bytes()
}

fn codex(session: &str, prompt: &str) -> Vec<u8> {
    format!(
        "{{\"session_id\":\"{session}\",\"turn_id\":\"t\",\"transcript_path\":null,\"cwd\":\"/h\",\"hook_event_name\":\"UserPromptSubmit\",\"model\":\"m\",\"permission_mode\":\"default\",\"prompt\":\"{prompt}\"}}"
    )
    .into_bytes()
}

fn files_only(scratch: &Scratch) {
    scratch.config(&closed(&format!(
        "{OFF}digest.markdown.most = 6\ndigest.session.most = 0\ndigest.note.most = 0\n"
    )));
}

#[test]
fn a_hooks_json_in_and_pointers_out() {
    let scratch = remembered("digest_pointers");
    files_only(&scratch);
    let run = scratch.run_with_input(&["digest"], &claude(OTHER, PROMPT));
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "files: 1 may bear on this prompt; bilbo read <ref> outlines it"
    );
    assert_eq!(lines.len(), 2, "{}", run.stdout);
    let row: Vec<&str> = lines[1].split('\t').collect();
    assert_eq!(row.len(), 3);
    let read = scratch.run(&["read", row[0]]);
    assert_eq!((read.code, read.stderr.as_str()), (0, ""));
    assert!(
        read.stdout.contains(&format!("label: {}", row[1])),
        "{}",
        read.stdout
    );
    assert!(
        read.stdout
            .lines()
            .any(|line| line.starts_with("#r") && line.ends_with(row[2])),
        "{}",
        read.stdout
    );
}

#[test]
fn nothing_on_a_sessions_second_prompt() {
    let scratch = remembered("digest_second");
    files_only(&scratch);
    let first = scratch.run_with_input(&["digest"], &claude(OTHER, PROMPT));
    assert!(first.stdout.starts_with("files:"));
    let again = scratch.run_with_input(&["digest"], &claude(OTHER, PROMPT));
    assert_eq!(
        (again.code, again.stdout.as_str(), again.stderr.as_str()),
        (0, "", "")
    );
    let other = scratch.run_with_input(
        &["digest"],
        &codex("22222222-3333-4444-8555-666666666666", PROMPT),
    );
    assert_eq!((other.code, other.stderr.as_str()), (0, ""));
    assert!(other.stdout.starts_with("files:"), "{}", other.stdout);
}

#[test]
fn exit_0_on_a_bad_json_a_missing_store_and_no_match() {
    let scratch = remembered("digest_bad");
    let bad = scratch.run_with_input(&["digest"], b"not json");
    assert_eq!((bad.code, bad.stdout.as_str()), (0, ""));
    assert_eq!(bad.stderr, "bilbo: the hook's input is not a JSON object\n");
    let empty = Scratch::new("digest_nostore");
    let missing = empty.run_with_input(&["digest"], &claude(OTHER, PROMPT));
    assert_eq!((missing.code, missing.stdout.as_str()), (0, ""));
    assert_eq!(
        missing.stderr,
        format!("bilbo: no store at {}\n", empty.path("store"))
    );
    let nothing = scratch.run_with_input(&["digest"], &claude(OTHER, "zzqqxx"));
    assert_eq!(
        (
            nothing.code,
            nothing.stdout.as_str(),
            nothing.stderr.as_str()
        ),
        (0, "", "")
    );
}

#[test]
fn an_unknown_option_and_a_config_error_exit_0() {
    let scratch = Scratch::new("digest_usage");
    let run = scratch.run(&["digest", "--all"]);
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(
        run.stderr,
        "bilbo: unknown option '--all'\nbilbo: usage: bilbo digest\nbilbo: see 'bilbo digest --help'\n"
    );
    scratch.config(&closed("digest.nonsense = 1\n"));
    let recall = scratch.run(&["recall", "garden"]);
    assert_eq!(recall.code, 2);
    let run = scratch.run_with_input(&["digest"], &claude(OTHER, PROMPT));
    assert_eq!((run.code, run.stdout.as_str()), (0, ""));
    assert_eq!(run.stderr, recall.stderr);
}

#[test]
fn each_rows_ref_opens_what_its_lists_line_says() {
    let scratch = remembered("digest_refs");
    let kept = scratch.run_with_input(
        &["remember", "-"],
        b"The carrots in the north bed need thinning on Monday.",
    );
    assert_eq!(kept.code, 0, "{}", kept.stderr);
    scratch.config(&closed(&format!(
        "{OFF}digest.markdown.most = 6\ndigest.session.most = 6\ndigest.note.most = 6\n"
    )));
    let run = scratch.run_with_input(&["digest"], &claude(OTHER, "garden beds carrots north"));
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    let mut at = "";
    let mut refs = std::collections::BTreeMap::new();
    for line in run.stdout.lines() {
        if line.contains("may bear on this prompt") {
            at = line.split(':').next().expect("a word");
        } else {
            refs.entry(at.to_owned())
                .or_insert_with(|| line.split('\t').next().expect("a ref").to_owned());
        }
    }
    assert_eq!(
        refs.keys().map(String::as_str).collect::<Vec<_>>(),
        ["files", "notes", "sessions"]
    );
    let file = scratch.run(&["read", &refs["files"]]);
    assert_eq!((file.code, file.stderr.as_str()), (0, ""));
    assert!(file.stdout.contains("outline:"), "{}", file.stdout);
    let session = scratch.run(&["read", &refs["sessions"]]);
    assert_eq!((session.code, session.stderr.as_str()), (0, ""));
    let (_, locator) = refs["sessions"].split_once("#r").expect("a locator");
    let (first, last) = locator.split_once('-').expect("a range");
    let (first, last): (u64, u64) = (
        first.parse().expect("a record"),
        last.parse().expect("a record"),
    );
    let records: Vec<u64> = session
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("#r"))
        .filter_map(|rest| rest.split([' ', '-']).next().and_then(|n| n.parse().ok()))
        .collect();
    assert!(!records.is_empty());
    assert!(
        records.iter().all(|record| (first..=last).contains(record)),
        "{records:?}"
    );
    assert!(records.contains(&first));
    assert_eq!((first, last), (3, 20), "the whole passage of the fixture");
    assert_eq!(records.iter().max(), Some(&20));
    assert!(
        session.stdout.contains("end: #r3-20 of 23 records"),
        "{}",
        session.stdout
    );
    let note = scratch.run(&["read", &refs["notes"]]);
    assert_eq!((note.code, note.stderr.as_str()), (0, ""));
    assert!(
        note.stdout
            .contains("The carrots in the north bed need thinning on Monday."),
        "{}",
        note.stdout
    );
}

fn unwritable(scratch: &Scratch, args: &[&str], input: &[u8]) -> (i32, String) {
    let readable_only = fs::File::open("/dev/null").expect("/dev/null opens");
    let mut command = Command::new(fs::canonicalize(binary()).expect("the binary"));
    command
        .env_clear()
        .envs(scratch.env())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(readable_only))
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("bilbo runs");
    std::io::Write::write_all(&mut child.stdin.take().expect("a pipe"), input).ok();
    let output = child.wait_with_output().expect("bilbo ends");
    (
        output.status.code().expect("an exit code"),
        String::from_utf8(output.stderr).expect("UTF-8 stderr"),
    )
}

#[test]
fn a_result_that_cannot_be_written_still_exits_0() {
    let scratch = remembered("digest_unwritable");
    files_only(&scratch);
    let held: Vec<(&str, Vec<u8>)> = vec![
        ("a list", claude(OTHER, PROMPT)),
        ("--help", Vec::new()),
        ("-h", Vec::new()),
    ];
    let (control_code, control) = unwritable(&scratch, &["help", "digest"], b"");
    assert_eq!(control_code, 1, "{control}");
    assert_eq!(
        control,
        "bilbo: cannot write to stdout: Bad file descriptor (os error 9)\n"
    );
    for (what, input) in held {
        let args: Vec<&str> = if what == "a list" {
            vec!["digest"]
        } else {
            vec!["digest", what]
        };
        let (code, stderr) = unwritable(&scratch, &args, &input);
        assert_eq!(code, 0, "{what}: {stderr}");
        assert_eq!(stderr, control, "{what}: the whole line, and no other");
    }
}
