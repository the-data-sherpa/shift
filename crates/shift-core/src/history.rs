//! What has happened to each finding across the rounds of a review: raised,
//! answered by the coder, resolved or still open. shift uses it to enforce
//! the round budget and Stuck; the reviewer pane shows it.

use serde::{Deserialize, Serialize};

use crate::delegate::{Answer, Finding, LedgerEntry, Review, Severity};

/// shift allows this many rounds per review.
pub const MAX_ROUNDS: u32 = 3;

/// A finding the coder has disagreed with this many times and the reviewer
/// still holds open is Stuck.
const MAX_DISPUTES: usize = 2;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    /// Rounds completed so far.
    pub rounds: u32,
    pub findings: Vec<FindingHistory>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FindingHistory {
    /// The finding as the reviewer last stated it.
    pub finding: Finding,
    pub open: bool,
    pub log: Vec<Entry>,
}

/// One step in a finding's history.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Raised { round: u32, claim: String },
    Answered { answer: Answer, note: String },
    StillOpen { round: u32, claim: String },
    Resolved { round: u32 },
}

impl History {
    /// Store the coder's ledger: its answer to each open finding. Every open
    /// finding must be answered, and only open findings can be.
    pub fn answer(&mut self, ledger: &[LedgerEntry]) -> Result<(), String> {
        for (i, entry) in ledger.iter().enumerate() {
            // One answer per finding: a repeat would count as a second reply.
            if ledger[..i].iter().any(|earlier| earlier.id == entry.id) {
                return Err(format!("the ledger answers `{}` more than once", entry.id));
            }
            let finding = self
                .findings
                .iter_mut()
                .find(|f| f.open && f.finding.id == entry.id)
                .ok_or_else(|| {
                    format!(
                        "the ledger answers `{}`, which is not an open finding",
                        entry.id
                    )
                })?;
            finding.log.push(Entry::Answered {
                answer: entry.answer,
                note: entry.note.clone(),
            });
        }
        let unanswered: Vec<&str> = self
            .findings
            .iter()
            .filter(|f| f.open && !matches!(f.log.last(), Some(Entry::Answered { .. })))
            .map(|f| f.finding.id.as_str())
            .collect();
        if unanswered.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "the ledger must answer every open finding; missing: {}",
                unanswered.join(", ")
            ))
        }
    }

    /// Record one round's review. An earlier open finding the review repeats
    /// is still open; one it leaves out is resolved.
    pub fn review(&mut self, review: &Review) {
        self.rounds += 1;
        let round = self.rounds;
        for old in self.findings.iter_mut().filter(|f| f.open) {
            match review.findings.iter().find(|f| f.id == old.finding.id) {
                Some(again) => {
                    old.finding = again.clone();
                    old.log.push(Entry::StillOpen {
                        round,
                        claim: again.claim.clone(),
                    });
                }
                None => {
                    old.open = false;
                    old.log.push(Entry::Resolved { round });
                }
            }
        }
        for finding in &review.findings {
            if !self
                .findings
                .iter()
                .any(|f| f.open && f.finding.id == finding.id)
            {
                self.findings.push(FindingHistory {
                    finding: finding.clone(),
                    open: true,
                    log: vec![Entry::Raised {
                        round,
                        claim: finding.claim.clone(),
                    }],
                });
            }
        }
    }

    /// Open findings that block approval: blockers and shoulds.
    fn blocking(&self) -> impl Iterator<Item = &FindingHistory> {
        self.findings
            .iter()
            .filter(|f| f.open && f.finding.severity != Severity::Nit)
    }

    /// A round that leaves only nits (or nothing) open ends the review.
    pub fn ended(&self) -> bool {
        self.blocking().next().is_none()
    }

    pub fn rounds_left(&self) -> u32 {
        MAX_ROUNDS.saturating_sub(self.rounds)
    }

    /// Why the review is Stuck right after a round, if it is: a finding is
    /// still open after the coder disagreed with it twice.
    /// `review` is the round that just completed.
    pub fn disputed(&self, review: &Review) -> Option<Stuck> {
        let disputes: Vec<Dispute> = self
            .blocking()
            .filter(|f| f.disagreements() >= MAX_DISPUTES)
            .map(FindingHistory::dispute)
            .collect();
        (!disputes.is_empty()).then(|| Stuck {
            review: Some(review.clone()),
            ..Stuck::new("A finding is still disputed after two replies.", disputes)
        })
    }

    /// Stuck because the round budget is spent, with every blocking finding.
    pub fn out_of_rounds(&self) -> Stuck {
        Stuck::new(
            format!("The round budget ({MAX_ROUNDS} rounds) is spent."),
            self.blocking().map(FindingHistory::dispute).collect(),
        )
    }
}

impl FindingHistory {
    fn disagreements(&self) -> usize {
        self.log
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Entry::Answered {
                        answer: Answer::Disagree,
                        ..
                    }
                )
            })
            .count()
    }

    fn dispute(&self) -> Dispute {
        Dispute {
            id: self.finding.id.clone(),
            file: self.finding.file.clone(),
            line: self.finding.line,
            reviewer: self.finding.claim.clone(),
            coder: self
                .log
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Answered { answer, note } => Some(format!("{}: {note}", answer.label())),
                    _ => None,
                })
                .collect(),
        }
    }
}

/// The end state when a finding stays disputed or the round budget runs
/// out. It goes to the user with both sides and never counts as approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stuck {
    /// Why the review is Stuck.
    pub stuck: String,
    pub disputes: Vec<Dispute>,
    /// What the coder must do now.
    pub next: String,
    /// The round that ended in Stuck, when one did: a spent budget completes
    /// no round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
}

impl Stuck {
    fn new(reason: impl Into<String>, disputes: Vec<Dispute>) -> Self {
        Self {
            stuck: reason.into(),
            disputes,
            next: "Stuck is not approval. Do not call delegate_task again. Stop and report \
                   both sides to the user."
                .into(),
            review: None,
        }
    }
}

/// Both sides of one finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dispute {
    pub id: String,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The reviewer's claim.
    pub reviewer: String,
    /// The coder's replies, oldest first.
    pub coder: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegate::ReviewVerdict;

    pub(crate) fn finding(id: &str, severity: Severity) -> Finding {
        Finding {
            id: id.into(),
            severity,
            file: "a.rs".into(),
            line: Some(3),
            claim: format!("{id} claim"),
            fix: "fix".into(),
        }
    }

    pub(crate) fn review(findings: Vec<Finding>) -> Review {
        let blocking = findings.iter().any(|f| f.severity != Severity::Nit);
        Review {
            verdict: if blocking {
                ReviewVerdict::Changes
            } else {
                ReviewVerdict::Approve
            },
            summary: "s".into(),
            findings,
            follow_ups: Vec::new(),
            commands_run: Vec::new(),
        }
    }

    fn entry(id: &str, answer: Answer) -> LedgerEntry {
        LedgerEntry {
            id: id.into(),
            answer,
            note: format!("{id} note"),
        }
    }

    #[test]
    fn findings_are_tracked_across_rounds() {
        let mut history = History::default();
        history.review(&review(vec![
            finding("S1", Severity::Should),
            finding("S2", Severity::Blocker),
        ]));
        assert!(!history.ended());

        history
            .answer(&[entry("S1", Answer::Fixed), entry("S2", Answer::Disagree)])
            .unwrap();
        // Round 2 repeats S2 with a new reason and raises S3; S1 is left out.
        let mut again = finding("S2", Severity::Blocker);
        again.claim = "still wrong".into();
        history.review(&review(vec![again, finding("S3", Severity::Nit)]));

        assert_eq!((history.rounds, history.rounds_left()), (2, 1));
        let log = |id: &str| {
            &history
                .findings
                .iter()
                .find(|f| f.finding.id == id)
                .unwrap()
                .log
        };
        assert_eq!(
            log("S1"),
            &[
                Entry::Raised {
                    round: 1,
                    claim: "S1 claim".into()
                },
                Entry::Answered {
                    answer: Answer::Fixed,
                    note: "S1 note".into()
                },
                Entry::Resolved { round: 2 },
            ]
        );
        assert_eq!(
            log("S2").last(),
            Some(&Entry::StillOpen {
                round: 2,
                claim: "still wrong".into()
            })
        );
        assert_eq!(
            log("S3"),
            &[Entry::Raised {
                round: 2,
                claim: "S3 claim".into()
            }]
        );
        assert!(!history.ended());
        assert_eq!(history.disputed(&review(Vec::new())), None);
    }

    #[test]
    fn a_round_with_only_nits_ends_the_review() {
        let mut history = History::default();
        history.review(&review(vec![finding("S1", Severity::Nit)]));
        assert!(history.ended());
        let mut history = History::default();
        history.review(&review(Vec::new()));
        assert!(history.ended());
    }

    #[test]
    fn ledger_must_answer_every_open_finding_and_nothing_else() {
        let mut history = History::default();
        history.review(&review(vec![
            finding("S1", Severity::Should),
            finding("S2", Severity::Nit),
        ]));

        let missing = history
            .clone()
            .answer(&[entry("S1", Answer::Fixed)])
            .unwrap_err();
        assert!(missing.contains("missing: S2"), "{missing}");
        let unknown = history
            .clone()
            .answer(&[entry("S9", Answer::Fixed)])
            .unwrap_err();
        assert!(unknown.contains("`S9`"), "{unknown}");
        assert!(history.clone().answer(&[]).is_err());
        // Answering S1 twice in one ledger is not two replies.
        let twice = [
            entry("S1", Answer::Disagree),
            entry("S1", Answer::Disagree),
            entry("S2", Answer::Declined),
        ];
        let repeated = history.clone().answer(&twice).unwrap_err();
        assert!(repeated.contains("more than once"), "{repeated}");
        assert!(
            history
                .answer(&[entry("S1", Answer::Fixed), entry("S2", Answer::Declined)])
                .is_ok()
        );
    }

    #[test]
    fn a_finding_disputed_twice_and_still_open_is_stuck() {
        let mut history = History::default();
        history.review(&review(vec![finding("S1", Severity::Should)]));
        history.answer(&[entry("S1", Answer::Disagree)]).unwrap();
        history.review(&review(vec![finding("S1", Severity::Should)]));
        assert_eq!(history.disputed(&review(Vec::new())), None);
        history.answer(&[entry("S1", Answer::Disagree)]).unwrap();
        let third = review(vec![finding("S1", Severity::Should)]);
        history.review(&third);

        let stuck = history.disputed(&third).unwrap();
        assert_eq!(stuck.review, Some(third));
        assert!(stuck.stuck.contains("disputed after two replies"));
        assert_eq!(stuck.disputes.len(), 1);
        assert_eq!(stuck.disputes[0].reviewer, "S1 claim");
        assert_eq!(
            stuck.disputes[0].coder,
            ["disagree: S1 note", "disagree: S1 note"]
        );
        assert!(stuck.next.contains("not approval"));
    }

    #[test]
    fn a_dispute_the_reviewer_drops_is_not_stuck() {
        let mut history = History::default();
        history.review(&review(vec![finding("S1", Severity::Should)]));
        history.answer(&[entry("S1", Answer::Disagree)]).unwrap();
        history.review(&review(vec![finding("S1", Severity::Should)]));
        history.answer(&[entry("S1", Answer::Disagree)]).unwrap();
        // The reviewer accepts the coder's evidence and leaves S1 out.
        history.review(&review(Vec::new()));
        assert_eq!(history.disputed(&review(Vec::new())), None);
        assert!(history.ended());
    }
}
