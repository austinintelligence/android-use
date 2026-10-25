//! Folding runs of look-alike rows (`Fixture row 1` .. `Fixture row 60`) in the compact listing.
use crate::format::{semantic_line, SemanticRow};

/// A row of the listing, or a run of look-alike rows (`Fixture row 1` .. `Fixture row 60`) shown as one line.
pub(crate) enum Part {
    Row(usize),
    More { pattern: String, count: usize },
}

impl Part {
    pub(crate) fn line(&self, rows: &[SemanticRow], numbers: &[Option<usize>]) -> String {
        match self {
            Self::Row(at) => semantic_line(&rows[*at], numbers[*at], false),
            Self::More { pattern, count } => format!("… +{count} more like \"{pattern}\" (screen full lists them)"),
        }
    }
}

/// `Fixture row 40` -> (`Fixture row`, plain), for rows that differ only by a trailing number.
fn run_key(row: &SemanticRow) -> Option<String> {
    let label = row.label.trim();
    let prefix = label.trim_end_matches(|c: char| c.is_ascii_digit());
    let plain = row.enabled && row.value.is_empty() && row.state.is_empty() && matches!(row.kind.as_str(), "" | "button" | "link");
    (plain && prefix.len() < label.len() && prefix.ends_with(' ') && prefix.trim().len() > 1 && !crate::resolve::risky(label)).then(|| format!("{}\0{}", row.kind, prefix.trim()))
}

/// Keeps the first two and the last row of every run of six or more look-alike rows. Empty when
/// nothing collapses, so the caller lists every row as before.
pub(crate) fn runs(rows: &[SemanticRow]) -> Vec<Part> {
    const MIN_RUN: usize = 6;
    let keys: Vec<Option<String>> = rows.iter().map(run_key).collect();
    let mut parts = Vec::new();
    let mut at = 0;
    let mut any = false;
    while at < rows.len() {
        let end = keys[at].as_ref().map_or(at + 1, |key| (at..rows.len()).take_while(|&i| keys[i].as_ref() == Some(key)).last().map_or(at + 1, |last| last + 1));
        if end - at >= MIN_RUN {
            any = true;
            let pattern = format!("{} N", rows[at].label.trim().trim_end_matches(|c: char| c.is_ascii_digit()).trim());
            parts.extend([Part::Row(at), Part::Row(at + 1), Part::More { pattern, count: end - at - 3 }, Part::Row(end - 1)]);
        } else {
            parts.extend((at..end).map(Part::Row));
        }
        at = end;
    }
    if any {
        parts
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::format_semantic_rows;

    fn sample(label: &str, kind: &str) -> SemanticRow {
        SemanticRow { label: label.into(), value: String::new(), kind: kind.into(), state: String::new(), enabled: true, selected: false, handle: None }
    }

    #[test]
    fn long_runs_of_look_alike_rows_collapse_and_screen_full_lists_them() {
        let mut rows = vec![sample("Benchmark scroll surface", "scroll area")];
        rows.extend((1..=60).map(|n| sample(&format!("Fixture row {n}"), "button")));
        rows.push(sample("Save", "button"));
        let compact = format_semantic_rows(&rows, false, "");
        assert!(compact.contains("Fixture row 1") && compact.contains("Fixture row 2") && compact.contains("Fixture row 60"), "{compact}");
        assert!(compact.contains("+57 more like \"Fixture row N\"") && !compact.contains("Fixture row 30") && compact.contains("Save"), "{compact}");
        assert!(format_semantic_rows(&rows, true, "").contains("Fixture row 30"));
        assert!(compact.len() * 3 < format_semantic_rows(&rows, true, "").len(), "the compact listing is much shorter");
    }

    #[test]
    fn risky_stateful_and_short_runs_are_listed_as_they_are() {
        let risky: Vec<SemanticRow> = (10..=20).map(|n| sample(&format!("Pay ${n}"), "button")).collect();
        assert!(!format_semantic_rows(&risky, false, "").contains("more like"));
        let short: Vec<SemanticRow> = (1..=5).map(|n| sample(&format!("Row {n}"), "button")).collect();
        assert!(!format_semantic_rows(&short, false, "").contains("more like"));
        let mut switches: Vec<SemanticRow> = (1..=9).map(|n| sample(&format!("Alarm {n}"), "switch")).collect();
        switches.iter_mut().for_each(|row| row.state = "on".into());
        assert!(!format_semantic_rows(&switches, false, "").contains("more like"), "switches carry state, so each stays visible");
        let mixed: Vec<SemanticRow> = (1..=9).map(|n| sample(&format!("Item {n}"), if n == 5 { "link" } else { "button" })).collect();
        assert!(!format_semantic_rows(&mixed, false, "").contains("more like"), "a different kind breaks the run");
    }
}
