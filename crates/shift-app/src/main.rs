//! shift desktop app: `shift <path>` opens a conversation with the coder in
//! that repo.

mod conversation;

use std::io;
use std::path::{Path, PathBuf};

use async_channel::Receiver;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use shift_adapter_claude::ClaudeAdapter;
use shift_core::{
    Adapter, Event, Session, WorkBranch, builtin_agent, new_session_dir, start_work_branch,
};

use conversation::Conversation;

/// Start the coder session in `cwd` on its own work branch, saving its
/// transcript under a new session directory. A dirty working tree is refused.
fn start_coder(cwd: &Path) -> io::Result<(WorkBranch, Box<dyn Session>, Receiver<Event>)> {
    let coder = builtin_agent("coder").map_err(io::Error::other)?;
    let (id, transcript_dir) = new_session_dir()?;
    let work = start_work_branch(cwd, &id[..8])?;
    let (session, events) = ClaudeAdapter.start(&coder, cwd, &transcript_dir)?;
    Ok((work, session, events))
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
    // `work.base` is what reviews will diff against.
    let (work, started) = match start_coder(&cwd) {
        Ok((work, session, events)) => (Some(work), Ok((session, events))),
        Err(e) => (None, Err(e)),
    };
    let branch = work.map(|work| work.branch);

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
            |window, cx| cx.new(|cx| Conversation::new(cwd, branch, started, window, cx)),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
