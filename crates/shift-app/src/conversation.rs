//! The conversation view: a live coder pane, the input that feeds it, and a
//! read-only reviewer pane.

use std::path::PathBuf;

use async_channel::Receiver;
use gpui_kit::component::{
    ActiveTheme, StyledExt, Theme, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::*;
use serde_json::Value;
use shift_core::{Event, Session};

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
}

pub struct Conversation {
    cwd: PathBuf,
    /// The session's `shift/<slug>` work branch.
    branch: Option<String>,
    session: Option<Box<dyn Session>>,
    pub items: Vec<Item>,
    /// The reviewer session. You talk to the coder; the coder talks to it.
    pub reviewer_items: Vec<Item>,
    input: Entity<InputState>,
    scroll: ScrollHandle,
    reviewer_scroll: ScrollHandle,
}

impl Conversation {
    /// `started` is the coder session, or why it could not start.
    /// `reviewer` carries the reviewer session's events.
    pub fn new(
        cwd: PathBuf,
        branch: Option<String>,
        started: std::io::Result<(Box<dyn Session>, Receiver<Event>)>,
        reviewer: Receiver<Event>,
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
                            apply(&mut this.items, event, "coder");
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
            input,
            scroll: ScrollHandle::new(),
            reviewer_scroll: ScrollHandle::new(),
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_owned();
        let Some(session) = &self.session else { return };
        if text.is_empty() {
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
                Input::new(&self.input)
                    .id("message")
                    .disabled(self.session.is_none()),
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
    use shift_core::{Event, Session};

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
                    Conversation::new("/repo".into(), None, started, reviewer, window, cx)
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
}
