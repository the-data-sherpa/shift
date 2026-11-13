//! shift desktop app: `shift <path>` opens a conversation with the coder in
//! that repo.

mod conversation;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_channel::Receiver;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use shift_adapter_claude::ClaudeAdapter;
use shift_adapter_codex::CodexReviewer;
use shift_core::review::{Delegate, UserRequest};
use shift_core::{
    Adapter, Cancel, Cancellable, Event, Session, builtin_agent, new_session_dir,
    start_work_branch, state_dir,
};

use conversation::Conversation;

/// What a started coder session hands the window.
struct Started {
    /// The session's `shift/<slug>` work branch.
    branch: String,
    coder: (Box<dyn Session>, Receiver<Event>),
    /// The reviewer session's events, for the read-only reviewer pane.
    reviewer: Receiver<Event>,
}

/// A coder session that remembers the user's original request, which every
/// review is checked against.
struct Recording {
    session: Box<dyn Session>,
    request: UserRequest,
}

impl Session for Recording {
    fn send(&self, text: &str) -> io::Result<()> {
        self.session.send(text)?;
        self.request
            .lock()
            .unwrap()
            .get_or_insert_with(|| text.to_owned());
        Ok(())
    }
}

/// Start the coder session in `cwd` on its own work branch, with
/// `delegate_task` backed by the reviewer. Transcripts are saved under a new
/// session directory. A dirty working tree is refused.
fn start_coder(cwd: &Path, cancel: Cancel) -> io::Result<Started> {
    let agent = |name| builtin_agent(name).map_err(io::Error::other);
    let (id, session_dir) = new_session_dir()?;
    let work = start_work_branch(cwd, &id[..8])?;

    let (reviewer_events, reviewer) = async_channel::unbounded();
    let request = UserRequest::default();
    let delegate = Delegate::new(
        cwd.to_owned(),
        work.base,
        session_dir.clone(),
        request.clone(),
        Box::new(CodexReviewer::new(
            agent("reviewer")?,
            // One build cache for every review, apart from the user's own.
            state_dir()?.join("reviewer-target"),
            &session_dir,
            reviewer_events,
            cancel.clone(),
        )?),
    );

    // Each `delegate_task` call is one review the user can cancel.
    let tools = Arc::new(Cancellable {
        tools: delegate,
        cancel,
    });
    let (session, events) = ClaudeAdapter.start(&agent("coder")?, cwd, &session_dir, tools)?;
    Ok(Started {
        branch: work.branch,
        coder: (Box::new(Recording { session, request }), events),
        reviewer,
    })
}

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let cwd = match path.canonicalize() {
        Ok(cwd) if cwd.is_dir() => cwd,
        _ => {
            eprintln!("shift: `{}` is not a directory", path.display());
            std::process::exit(2);
        }
    };
    // The Cancel review button and the reviewer share this.
    let cancel = Cancel::default();
    let (branch, started, reviewer) = match start_coder(&cwd, cancel.clone()) {
        Ok(started) => (Some(started.branch), Ok(started.coder), started.reviewer),
        // No session: the reviewer pane stays empty.
        Err(e) => (None, Err(e), async_channel::unbounded().1),
    };

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        let bounds = Bounds::centered(None, size(px(900.), px(700.)), cx);
        open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("shift".into()),
                    ..Default::default()
                }),
                app_id: Some("shift".into()),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| Conversation::new(cwd, branch, started, reviewer, cancel, window, cx))
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
