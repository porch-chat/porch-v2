//! Native Evening Threshold primitives, independent of network state.
use super::*;
use chrono::{DateTime, Local};
use gpui_component::Icon;

pub fn icon(name: &str) -> Icon {
    Icon::default().path(SharedString::from(format!("porch/{name}.svg")))
}
pub fn avatar(id: i64, name: &str, size: f32) -> Div {
    let colors = [0x435846, 0x674D3F, 0x435762, 0x61516A, 0x655D3B, 0x3D615B];
    let initials = name
        .split_whitespace()
        .take(2)
        .filter_map(|s| s.chars().next())
        .collect::<String>()
        .to_uppercase();
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(size * 0.36))
        .bg(rgb(colors[id.unsigned_abs() as usize % colors.len()]))
        .text_color(rgb(CREAM))
        .text_size(px(size * 0.34))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initials)
}
fn parse_time(value: &str) -> Result<DateTime<chrono::FixedOffset>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f%#z"))
}
pub fn clock(value: &str) -> String {
    parse_time(value)
        .map(|t| t.with_timezone(&Local).format("%-I:%M %p").to_string())
        .unwrap_or_else(|_| value.to_string())
}
pub fn grouped(previous: &Message, next: &Message) -> bool {
    if previous.author_id != next.author_id || previous.deleted || next.reply_to.is_some() {
        return false;
    }
    match (
        parse_time(&previous.created_at),
        parse_time(&next.created_at),
    ) {
        (Ok(a), Ok(b)) => {
            let seconds = (b - a).num_seconds();
            (0..300).contains(&seconds) && a.date_naive() == b.date_naive()
        }
        _ => false,
    }
}
impl Porch {
    pub(super) fn member_name(&self, id: i64) -> String {
        self.snapshot
            .user
            .as_ref()
            .filter(|u| u.id == id)
            .map(|u| u.username.clone())
            .or_else(|| {
                self.snapshot
                    .communities
                    .iter()
                    .find_map(|c| c.usernames.get(&id).cloned())
            })
            .unwrap_or_else(|| format!("User {id}"))
    }
    pub(super) fn user_panel(&self, cx: &mut Context<Self>) -> Div {
        let id = self
            .snapshot
            .user
            .as_ref()
            .map(|u| u.id)
            .unwrap_or_default();
        div()
            .px_2()
            .h(px(56.))
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0x20271F))
            .flex()
            .items_center()
            .gap_2()
            .child(avatar(id, &self.member_name(id), 34.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(heading(self.member_name(id)).text_size(px(12.)))
                    .child(small(self.snapshot.state.clone()).text_size(px(10.))),
            )
            .child(
                Button::new("panel-mic")
                    .ghost()
                    .xsmall()
                    .icon(icon(if self.snapshot.voice.manual_muted {
                        "mic-off"
                    } else {
                        "mic"
                    }))
                    .tooltip("Toggle microphone")
                    .disabled(self.snapshot.voice.channel.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let v = &this.snapshot.voice;
                        this.command(
                            Command::VoiceFlags {
                                muted: !v.manual_muted,
                                deafened: v.flags.deafened,
                                ptt: v.ptt,
                                held: false,
                            },
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("panel-deafen")
                    .ghost()
                    .xsmall()
                    .icon(icon("headphones"))
                    .tooltip("Toggle deafen")
                    .disabled(self.snapshot.voice.channel.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let v = &this.snapshot.voice;
                        this.command(
                            Command::VoiceFlags {
                                muted: v.manual_muted,
                                deafened: !v.flags.deafened,
                                ptt: v.ptt,
                                held: false,
                            },
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("user-settings")
                    .ghost()
                    .xsmall()
                    .icon(icon("settings"))
                    .tooltip("Settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_settings = !this.show_settings;
                        cx.notify();
                    })),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn message(author: i64, time: &str) -> Message {
        serde_json::from_value(serde_json::json!({
            "id":1,"conversation_id":1,"author_id":author,"nonce":"fixture",
            "content":{"format":"markdown","text":"hello"},"rendered_html":null,
            "reply_to":null,"mentions":[],"mention_everyone":false,"created_at":time,
            "edited_at":null,"deleted":false,"pinned":false,"reactions":[],"revision":1
        }))
        .unwrap()
    }
    #[::core::prelude::v1::test]
    fn grouping_preserves_author_time_and_reply_boundaries() {
        let first = message(1, "2026-11-06 13:00:00+00");
        let mut next = message(1, "2026-11-06 13:04:59+00");
        assert!(grouped(&first, &next));
        next.created_at = "2026-11-06 13:05:00+00".into();
        assert!(!grouped(&first, &next));
        next.created_at = first.created_at.clone();
        next.author_id = 2;
        assert!(!grouped(&first, &next));
        next.author_id = 1;
        next.reply_to = Some(99);
        assert!(!grouped(&first, &next));
    }
    #[::core::prelude::v1::test]
    fn postgres_timestamp_formats_as_a_clock() {
        assert!(parse_time("2026-11-06 13:00:53.839435+00").is_ok());
        assert!(!clock("2026-11-06 13:00:53.839435+00").contains("2026"));
    }
    #[::core::prelude::v1::test]
    fn timestamp_fallback_preserves_unrecognized_value() {
        assert_eq!(clock("unknown"), "unknown");
    }
    #[::core::prelude::v1::test]
    fn timestamp_is_short_and_has_no_raw_iso_date() {
        assert!(!clock("2026-11-06T12:30:00Z").contains("2026"));
    }
}
