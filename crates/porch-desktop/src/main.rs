#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod assets;
mod session;
mod settings;
mod view;
mod voice;
use gpui::{prelude::*, *};
use gpui_component::{Root, Theme, ThemeMode};
use std::borrow::Cow;
fn main() {
    let root = match session::data_dir() {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Porch cannot open its private local data directory.");
            std::process::exit(1)
        }
    };
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            let theme = Theme::global_mut(cx);
            theme.font_family = "Manrope".into();
            theme.background = rgb(0x111210).into();
            theme.foreground = rgb(0xF4E8D7).into();
            theme.primary = rgb(0xF3B85A).into();
            theme.primary_foreground = rgb(0x241B0F).into();
            theme.primary_hover = rgb(0xFFCD7D).into();
            theme.primary_active = rgb(0xF3B85A).into();
            theme.secondary = rgb(0x232B24).into();
            theme.secondary_foreground = rgb(0xF4E8D7).into();
            theme.muted = rgb(0x232B24).into();
            theme.muted_foreground = rgb(0xB8B5A6).into();
            theme.input = rgb(0x596458).into();
            theme.border = rgb(0x596458).into();
            theme.caret = rgb(0xF3B85A).into();
            theme.ring = rgb(0xF3B85A).into();
            cx.text_system()
                .add_fonts(vec![Cow::Borrowed(include_bytes!(
                    "../../../assets/brand/fonts/Manrope.ttf"
                ))])
                .expect("embedded Manrope");
            let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(950.), px(640.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Porch".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    let view = cx.new(|cx| view::Porch::new(root, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("open Porch window");
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
            // Only explicitly requested smoke exits automatically. No user session is killed.
            if let Some(seconds) = std::env::var("PORCH_SMOKE_SECONDS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| (1..=120).contains(n))
            {
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(seconds))
                        .await;
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
}
