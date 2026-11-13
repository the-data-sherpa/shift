//! The conversation view: a live coder pane, the input that feeds it, and a
//! read-only reviewer pane.

use std::collections::HashMap;
use std::path::PathBuf;

use async_channel::Receiver;
use gpui_kit::component::{
    ActiveTheme, StyledExt, Theme,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::*;
use serde_json::Value;
use shift_core::delegate::LedgerEntry;
use shift_core::history::{Entry, History, Stuck};
use shift_core::review::RoundResult;
use shift_core::{Cancel, Event, Session};

/// `delegate_task` as Claude names it.
const DELEGATE_TASK: &str = "mcp__shift__delegate_task";

/// One entry in a pane.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    User(String),
    Agent(String),
    Tool {
        id: String,
        name: String,
        input: String,
        /// Output and whether it is an error, once the tool has finished.
        result: Option<(String, bool)>,
    },
    Error(String),
    /// The review ended Stuck. shift shows both sides itself.
    Stuck(Stuck),
}

pub struct Conversation {
    cwd: PathBuf,
    /// The session's `shift/<slug>` work branch.
    branch: Option<String>,
    session: Option<Box<dyn Session>>,
    pub items: Vec<Item>,
    /// The reviewer session. You talk to the coder; the coder talks to it.
    pub reviewer_items: Vec<Item>,
    /// Each finding's history across rounds, shown in the reviewer pane.
    pub history: History,
    /// `delegate_task` calls that have not returned, by tool call id.
    reviewing: HashMap<String, Value>,
    input: Entity<InputState>,
    scroll: ScrollHandle,
    reviewer_scroll: ScrollHandle,
    /// Stops the running review.
    cancel: Cancel,
}

impl Conversation {
    /// `started` is the coder session, or why it could not start.
    /// `reviewer` carries the reviewer session's events.
    pub fn new(
        cwd: PathBuf,
        branch: Option<String>,
        started: std::io::Result<(Box<dyn Session>, Receiver<Event>)>,
        reviewer: Receiver<Event>,
        cancel: Cancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Message the coder"));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(window, cx);
            }
        })
        .detach();
        input.update(cx, |input, cx| input.focus(window, cx));

        let mut items = Vec::new();
        let session = match started {
            Ok((session, events)) => {
                cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv().await {
                        let updated = this.update(cx, |this, cx| {
                            if event == Event::Closed {
                                this.session = None;
                            }
                            let stuck = this.track_review(&event);
                            apply(&mut this.items, event, "coder");
                            this.items.extend(stuck.map(Item::Stuck));
                            this.scroll.scroll_to_bottom();
                            cx.notify();
                        });
                        if updated.is_err() {
                            break;
                        }
                    }
                })
                .detach();
                Some(session)
            }
            Err(e) => {
                items.push(Item::Error(format!("Could not start the coder: {e}")));
                None
            }
        };

        cx.spawn(async move |this, cx| {
            while let Ok(event) = reviewer.recv().await {
                let updated = this.update(cx, |this, cx| {
                    apply(&mut this.reviewer_items, event, "reviewer");
                    this.reviewer_scroll.scroll_to_bottom();
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            cwd,
            branch,
            session,
            items,
            reviewer_items: Vec::new(),
            history: History::default(),
            reviewing: HashMap::new(),
            input,
            scroll: ScrollHandle::new(),
            reviewer_scroll: ScrollHandle::new(),
            cancel,
        }
    }

    /// The id of the coder's `delegate_task` call that has no result yet:
    /// a review runs from that call until its result.
    fn running_review(&self) -> Option<&str> {
        self.items.iter().find_map(|item| match item {
            Item::Tool {
                id,
                name,
                result: None,
                ..
            } if name == DELEGATE_TASK => Some(id.as_str()),
            _ => None,
        })
    }

    pub fn reviewing(&self) -> bool {
        self.running_review().is_some()
    }

    /// The Cancel review button: stop the review this pane shows as
    /// running. The coder gets a "cancelled by user" tool error.
    pub fn cancel_review(&mut self) {
        if let Some(call_id) = self.running_review() {
            self.cancel.cancel(call_id);
        }
    }

    /// Follow `delegate_task` calls through the coder's events: a returned
    /// round updates the finding history, and Stuck is handed back to show.
    fn track_review(&mut self, event: &Event) -> Option<Stuck> {
        match event {
            Event::ToolCall { id, name, input } if name == DELEGATE_TASK => {
                self.reviewing.insert(id.clone(), input.clone());
                None
            }
            Event::ToolResult {
                id,
                output,
                is_error,
            } => {
                let input = self.reviewing.remove(id)?;
                if *is_error {
                    // A failed review changed nothing.
                    return None;
                }
                // Once Stuck, shift repeats the same result and runs no more
                // rounds, so the history is final.
                if self.items.iter().any(|item| matches!(item, Item::Stuck(_))) {
                    return serde_json::from_str(output).ok();
                }
                let ledger: Vec<LedgerEntry> =
                    serde_json::from_value(input["ledger"].clone()).unwrap_or_default();
                // shift already accepted this ledger, so it fits the history.
                let _ = self.history.answer(&ledger);
                if let Ok(stuck) = serde_json::from_str::<Stuck>(output) {
                    // A dispute ends in Stuck on a completed round.
                    if let Some(review) = &stuck.review {
                        self.history.review(review);
                    }
                    return Some(stuck);
                }
                if let Ok(round) = serde_json::from_str::<RoundResult>(output) {
                    self.history.review(&round.review);
                }
                None
            }
            _ => None,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_owned();
        let Some(session) = &self.session else { return };
        if text.is_empty() || self.reviewing() {
            return;
        }
        match session.send(&text) {
            Ok(()) => self.items.push(Item::User(text)),
            Err(e) => self.items.push(Item::Error(format!("Could not send: {e}"))),
        }
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.scroll.scroll_to_bottom();
        cx.notify();
    }
}

/// Fold one session event into a pane's items.
fn apply(items: &mut Vec<Item>, event: Event, agent: &str) {
    match event {
        Event::Text(text) => match items.last_mut() {
            Some(Item::Agent(reply)) => reply.push_str(&text),
            _ => items.push(Item::Agent(text)),
        },
        Event::ToolCall { id, name, input } => items.push(Item::Tool {
            id,
            name,
            input: tool_input(&input),
            result: None,
        }),
        Event::ToolResult {
            id,
            output,
            is_error,
        } => {
            if let Some(Item::Tool { result, .. }) = items
                .iter_mut()
                .rev()
                .find(|item| matches!(item, Item::Tool { id: call, .. } if *call == id))
            {
                *result = Some((output, is_error));
            }
        }
        Event::PermissionRequest { tool, allowed } => {
            if !allowed {
                items.push(Item::Error(format!(
                    "Denied `{tool}`: not in the {agent}'s tools."
                )));
            }
        }
        Event::Error(text) => items.push(Item::Error(text)),
        Event::TurnDone { .. } => {}
        Event::Closed => items.push(Item::Error(format!("The {agent} session ended."))),
    }
}

/// A one-line view of a tool call's input: a shell command, a file path, or
/// a `delegate_task` intent; otherwise the JSON.
fn tool_input(input: &Value) -> String {
    match input["command"]
        .as_str()
        .or(input["file_path"].as_str())
        .or(input["intent"].as_str())
    {
        Some(text) => text.to_owned(),
        None => input.to_string(),
    }
}

/// Long tool output would bury the conversation; show its head.
fn head(text: &str) -> String {
    const LINES: usize = 12;
    let mut lines: Vec<&str> = text.lines().take(LINES + 1).collect();
    if lines.len() > LINES {
        lines.truncate(LINES);
        lines.push("…");
    }
    lines.join("\n")
}

fn render_items(items: &[Item], theme: &Theme) -> Vec<Div> {
    items
        .iter()
        .map(|item| match item {
            Item::User(text) => div()
                .p_2()
                .rounded(theme.radius)
                .bg(theme.secondary)
                .child(text.clone()),
            Item::Agent(text) => div().child(text.clone()),
            Item::Tool {
                name,
                input,
                result,
                ..
            } => v_flex()
                .gap_1()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .text_sm()
                .font_family(theme.mono_font_family.clone())
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().font_semibold().child(name.clone()))
                        .child(div().child(input.clone())),
                )
                .child(match result {
                    None => div().text_color(theme.muted_foreground).child("running…"),
                    Some((output, is_error)) => div()
                        .text_color(if *is_error {
                            theme.danger
                        } else {
                            theme.muted_foreground
                        })
                        .child(head(output)),
                }),
            Item::Error(text) => div().text_color(theme.danger).child(text.clone()),
            Item::Stuck(stuck) => v_flex()
                .gap_2()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.danger)
                .child(
                    div()
                        .font_semibold()
                        .text_color(theme.danger)
                        .child(format!("Stuck: {} This is not an approval.", stuck.stuck)),
                )
                .children(stuck.disputes.iter().map(|dispute| {
                    v_flex()
                        .gap_1()
                        .child(div().font_semibold().child(format!(
                            "{} {}",
                            dispute.id,
                            location(&dispute.file, dispute.line)
                        )))
                        .child(div().child(format!("Reviewer: {}", dispute.reviewer)))
                        .children(
                            dispute
                                .coder
                                .iter()
                                .map(|reply| div().child(format!("Coder: {reply}"))),
                        )
                })),
        })
        .collect()
}

fn location(file: &str, line: Option<u32>) -> String {
    match line {
        Some(line) => format!("{file}:{line}"),
        None => file.to_owned(),
    }
}

/// Each finding with what happened to it, round by round.
fn render_history(history: &History, theme: &Theme) -> Vec<Div> {
    history
        .findings
        .iter()
        .map(|finding| {
            let state = if finding.open { "open" } else { "resolved" };
            v_flex()
                .gap_1()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .text_sm()
                .child(div().font_semibold().child(format!(
                    "{} {} {} ({state})",
                    finding.finding.id,
                    format!("{:?}", finding.finding.severity).to_lowercase(),
                    location(&finding.finding.file, finding.finding.line),
                )))
                .children(finding.log.iter().map(|entry| {
                    div().text_color(theme.muted_foreground).child(match entry {
                        Entry::Raised { round, claim } => format!("Round {round}: raised. {claim}"),
                        Entry::Answered { answer, note } => {
                            format!("Coder: {}. {note}", answer.label())
                        }
                        Entry::StillOpen { round, claim } => {
                            format!("Round {round}: still open. {claim}")
                        }
                        Entry::Resolved { round } => format!("Round {round}: resolved."),
                    })
                }))
        })
        .collect()
}

impl Render for Conversation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = |text: String| {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
        };

        let coder = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .overflow_hidden()
                    .child(div().font_semibold().child("coder"))
                    .child(muted(self.cwd.display().to_string()))
                    .children(self.branch.clone().map(muted)),
            )
            .child(
                v_flex()
                    .id("coder-pane")
                    .flex_1()
                    .gap_2()
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .children(render_items(&self.items, theme)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div().flex_1().child(
                            Input::new(&self.input)
                                .id("message")
                                .disabled(self.session.is_none() || self.reviewing()),
                        ),
                    )
                    .children(self.reviewing().then(|| {
                        let button = div().child(
                            Button::new("cancel-review")
                                .label("Cancel review")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_review();
                                    cx.notify();
                                })),
                        );
                        // Lets the UI test find the button and click it.
                        #[cfg(test)]
                        let button = button.debug_selector(|| "cancel-review".into());
                        button
                    })),
            );

        // The reviewer pane appears with the first review and has no input.
        let reviewer = (!self.reviewer_items.is_empty()).then(|| {
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .gap_3()
                .pl_4()
                .border_l_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().font_semibold().child("reviewer"))
                        .child(muted("read-only".into())),
                )
                .children((!self.history.findings.is_empty()).then(|| {
                    v_flex()
                        .id("finding-history")
                        .max_h(px(280.))
                        .gap_2()
                        .overflow_y_scroll()
                        .children(render_history(&self.history, theme))
                }))
                .child(
                    v_flex()
                        .id("reviewer-pane")
                        .flex_1()
                        .gap_2()
                        .track_scroll(&self.reviewer_scroll)
                        .overflow_y_scroll()
                        .children(render_items(&self.reviewer_items, theme)),
                )
        });

        h_flex()
            .size_full()
            .gap_4()
            .p_4()
            .items_start()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(coder)
            .children(reviewer)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gpui::{AppContext, TestAppContext, VisualTestContext, WindowOptions};
    use serde_json::json;
    use shift_core::delegate::Answer;
    use shift_core::history::Entry;
    use shift_core::{Cancel, Event, Session};

    // No `use super::*`: GPUI's glob would shadow Rust's `#[test]`.
    use super::{Conversation, Item};

    struct FakeSession(Arc<Mutex<Vec<String>>>);

    impl Session for FakeSession {
        fn send(&self, text: &str) -> std::io::Result<()> {
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    #[gpui::test]
    fn typed_message_reaches_the_coder_and_its_events_show_in_the_pane(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (events, receiver) = async_channel::unbounded();
        let (reviewer_events, reviewer) = async_channel::unbounded();
        let session: Box<dyn Session> = Box::new(FakeSession(sent.clone()));
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let started = Ok((session, receiver));
                    Conversation::new(
                        "/repo".into(),
                        None,
                        started,
                        reviewer,
                        Cancel::default(),
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });

        let cx = &mut VisualTestContext::from_window(window, cx);

        cx.simulate_input("fix the bug");
        cx.simulate_keystrokes("enter");
        assert_eq!(*sent.lock().unwrap(), ["fix the bug"]);

        for event in [
            Event::Text("On ".into()),
            Event::Text("it.".into()),
            Event::ToolCall {
                id: "t1".into(),
                name: "Bash".into(),
                input: json!({"command": "cargo test"}),
            },
            Event::ToolResult {
                id: "t1".into(),
                output: "ok".into(),
                is_error: false,
            },
            Event::Text("Fixed.".into()),
        ] {
            events.send_blocking(event).unwrap();
        }
        // The reviewer session streams into its own pane.
        for event in [
            Event::Text("Reading the diff.".into()),
            Event::ToolCall {
                id: "r1".into(),
                name: "shell".into(),
                input: json!({"command": "git diff BASE"}),
            },
        ] {
            reviewer_events.send_blocking(event).unwrap();
        }
        cx.run_until_parked();

        cx.update(|_, cx| {
            assert_eq!(
                view.read(cx).reviewer_items,
                [
                    Item::Agent("Reading the diff.".into()),
                    Item::Tool {
                        id: "r1".into(),
                        name: "shell".into(),
                        input: "git diff BASE".into(),
                        result: None,
                    },
                ]
            );
            assert_eq!(view.read(cx).input.read(cx).value(), "");
            assert_eq!(
                view.read(cx).items,
                [
                    Item::User("fix the bug".into()),
                    Item::Agent("On it.".into()),
                    Item::Tool {
                        id: "t1".into(),
                        name: "Bash".into(),
                        input: "cargo test".into(),
                        result: Some(("ok".into(), false)),
                    },
                    Item::Agent("Fixed.".into()),
                ]
            );
        });
    }

    #[gpui::test]
    fn reviews_build_the_finding_history_and_stuck_shows_both_sides(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (events, receiver) = async_channel::unbounded();
        let (_reviewer_events, reviewer) = async_channel::unbounded();
        let session: Box<dyn Session> = Box::new(FakeSession(Default::default()));
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let started = Ok((session, receiver));
                    Conversation::new(
                        "/repo".into(),
                        None,
                        started,
                        reviewer,
                        Cancel::default(),
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let cx = &mut VisualTestContext::from_window(window, cx);
        let call = |id: &str, ledger: serde_json::Value| Event::ToolCall {
            id: id.into(),
            name: "mcp__shift__delegate_task".into(),
            input: json!({"intent": "i", "ledger": ledger}),
        };
        let result = |id: &str, output: serde_json::Value, is_error: bool| Event::ToolResult {
            id: id.into(),
            output: output.to_string(),
            is_error,
        };
        let round = json!({
            "verdict": "changes", "summary": "s", "follow_ups": [], "commands_run": [],
            "findings": [{"id": "S1", "severity": "should", "file": "a.rs", "line": 3, "claim": "wrong", "fix": "f"}],
            "round": 1, "rounds_left": 2, "ended": false,
        });
        let disagree = json!([{"id": "S1", "answer": "disagree", "note": "see the test"}]);
        let stuck = json!({
            "stuck": "A finding is still disputed after two replies.",
            "disputes": [{"id": "S1", "file": "a.rs", "line": 3, "reviewer": "wrong", "coder": ["disagree: see the test"]}],
            "next": "Stop.",
            "review": {
                "verdict": "changes", "summary": "s", "follow_ups": [], "commands_run": [],
                "findings": [{"id": "S1", "severity": "should", "file": "a.rs", "line": 3, "claim": "still wrong", "fix": "f"}],
            },
        });

        for event in [
            call("t1", json!(null)),
            result("t1", round, false),
            // A failed review changes nothing.
            call("t2", disagree.clone()),
            result("t2", json!("the review exceeded its limit"), true),
            call("t3", disagree.clone()),
            result("t3", stuck.clone(), false),
            // Stuck stays Stuck: a further call adds no round to the history.
            call("t4", disagree),
            result("t4", stuck, false),
        ] {
            events.send_blocking(event).unwrap();
        }
        cx.run_until_parked();

        cx.update(|_, cx| {
            let view = view.read(cx);
            let finding = &view.history.findings[0];
            assert_eq!((finding.finding.id.as_str(), finding.open), ("S1", true));
            assert_eq!(
                finding.log,
                [
                    Entry::Raised {
                        round: 1,
                        claim: "wrong".into()
                    },
                    Entry::Answered {
                        answer: Answer::Disagree,
                        note: "see the test".into()
                    },
                    // The round that ended in Stuck is in the history too.
                    Entry::StillOpen {
                        round: 2,
                        claim: "still wrong".into()
                    },
                ]
            );
            // Stuck is shown by shift itself, after the call that returned it.
            let Some(Item::Stuck(shown)) = view.items.last() else {
                panic!("no Stuck item: {:?}", view.items);
            };
            assert_eq!(shown.disputes[0].reviewer, "wrong");
            assert_eq!(shown.disputes[0].coder, ["disagree: see the test"]);
        });
    }

    #[gpui::test]
    fn a_running_review_disables_the_input_and_can_be_cancelled(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (events, receiver) = async_channel::unbounded();
        let (_reviewer_events, reviewer) = async_channel::unbounded();
        let session: Box<dyn Session> = Box::new(FakeSession(sent.clone()));
        let cancel = Cancel::default();
        let button = cancel.clone();
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let started = Ok((session, receiver));
                    Conversation::new("/repo".into(), None, started, reviewer, button, window, cx)
                })
            })
            .unwrap()
        });
        let cx = &mut VisualTestContext::from_window(window, cx);
        let review = |id: &str| Event::ToolCall {
            id: id.into(),
            name: "mcp__shift__delegate_task".into(),
            input: json!({"intent": "fix the bug"}),
        };
        let cancelled = |id: &str| Event::ToolResult {
            id: id.into(),
            output: "cancelled by user".into(),
            is_error: true,
        };

        // No review is running: there is nothing to cancel.
        cancel.begin("d0");
        view.update(cx, |view, _| view.cancel_review());
        assert!(!cancel.is_cancelled());

        events.send_blocking(review("d1")).unwrap();
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.reviewing()));
        cancel.begin("d1");

        // The input takes nothing while the review runs.
        cx.simulate_input("hello");
        cx.simulate_keystrokes("enter");
        assert!(sent.lock().unwrap().is_empty());

        // Click the Cancel review button.
        let button = cx.debug_bounds("cancel-review").expect("the button shows");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        assert!(cancel.is_cancelled());

        // The pane lags: shift is already running the next review when the
        // button, still showing for d1, is pressed. Only d1 is cancelled.
        cancel.begin("d2");
        view.update(cx, |view, _| view.cancel_review());
        assert!(!cancel.is_cancelled());

        // The cancelled review comes back as a tool error; the input works again.
        events.send_blocking(cancelled("d1")).unwrap();
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.reviewing()));
        assert!(cx.debug_bounds("cancel-review").is_none());
        cx.simulate_input("try again");
        cx.simulate_keystrokes("enter");
        assert_eq!(*sent.lock().unwrap(), ["try again"]);
    }
}
