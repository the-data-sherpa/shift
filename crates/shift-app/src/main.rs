//! shift desktop app: `shift <path>` opens a conversation with the coder in
//! that repo.

mod conversation;

use std::io;
use std::path::{Path, PathBuf};

use async_channel::Receiver;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use shift_adapter_claude::ClaudeAdapter;
use shift_core::{Adapter, Event, Session, builtin_agent, new_session_dir};

use conversation::Conversation;

/// Start the coder session in `cwd`, saving its transcript under a new
/// session directory.
fn start_coder(cwd: &Path) -> io::Result<(Box<dyn Session>, Receiver<Event>)> {
    let coder = builtin_agent("coder").map_err(io::Error::other)?;
    let (_, transcript_dir) = new_session_dir()?;
    ClaudeAdapter.start(&coder, cwd, &transcript_dir)
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
    let started = start_coder(&cwd);

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
            |window, cx| cx.new(|cx| Conversation::new(cwd, started, window, cx)),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
