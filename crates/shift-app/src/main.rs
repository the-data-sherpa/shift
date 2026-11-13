//! shift desktop app. For now: a window listing the loaded agent definitions.

use std::path::Path;

use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, Theme, ThemeMode, h_flex, scroll::ScrollableElement,
    tag::Tag, v_flex,
};
use gpui_kit::*;
use shift_core::{AgentDef, Harness, load_agents_dir};

struct Roster {
    agents: Vec<AgentDef>,
    error: Option<SharedString>,
}

fn harness_tag(harness: Harness) -> Tag {
    let tag = match harness {
        Harness::ClaudeCode => Tag::warning(),
        Harness::Codex => Tag::info(),
        Harness::Antigravity => Tag::success(),
    };
    tag.outline().child(harness.label())
}

impl Render for Roster {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let cards = self.agents.iter().map(|agent| {
            v_flex()
                .gap_1()
                .p_3()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().font_semibold().child(agent.name.clone()))
                        .child(harness_tag(agent.harness)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(agent.description.clone()),
                )
                .child(
                    h_flex().gap_1().flex_wrap().children(
                        agent
                            .skills
                            .iter()
                            .map(|s| Tag::secondary().small().child(s.clone())),
                    ),
                )
        });

        v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(div().text_xl().font_semibold().child("shift — agents"))
            .children(self.error.clone().map(|e| div().text_color(theme.danger).child(e)))
            .child(
                v_flex()
                    .id("agents")
                    .flex_1()
                    .gap_2()
                    .overflow_y_scrollbar()
                    .children(cards),
            )
    }
}

fn main() {
    let (agents, error) = match load_agents_dir(Path::new("agents")) {
        Ok(a) => (a, None),
        Err(e) => (Vec::new(), Some(e.to_string().into())),
    };

    application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            init(cx);
            Theme::change(ThemeMode::Dark, None, cx);

            let bounds = Bounds::centered(None, size(px(640.), px(480.)), cx);
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
                |_, cx| cx.new(|_| Roster { agents, error }),
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}
