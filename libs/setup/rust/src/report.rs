#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Installed,
    Updated,
    Kept,
    Removed,
    Absent,
    Skipped,
    Failed,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Installed => "installed",
            Status::Updated => "updated",
            Status::Kept => "kept",
            Status::Removed => "removed",
            Status::Absent => "absent",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub step: &'static str,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub lines: Vec<Line>,
}

impl Report {
    pub fn push(&mut self, step: &'static str, status: Status, detail: String) {
        self.lines.push(Line {
            step,
            status,
            detail,
        });
    }

    pub fn failed(&self) -> bool {
        self.lines.iter().any(|line| line.status == Status::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Report, Status};

    #[test]
    fn a_report_failed_when_a_line_did() {
        let words = [
            (Status::Installed, "installed"),
            (Status::Updated, "updated"),
            (Status::Kept, "kept"),
            (Status::Skipped, "skipped"),
            (Status::Failed, "failed"),
        ];
        for (status, word) in words {
            assert_eq!(status.word(), word);
        }
        let mut report = Report::default();
        assert!(!report.failed());
        for (status, _) in &words[..4] {
            report.push("step", *status, String::new());
        }
        assert!(!report.failed());
        report.push("step", Status::Failed, "why".to_owned());
        assert!(report.failed());
        assert_eq!(report.lines.len(), 5);
        assert_eq!(report.lines[4].detail, "why");
    }
}
