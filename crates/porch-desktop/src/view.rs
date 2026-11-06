mod design;
mod media;
use crate::{
    session::{Bridge, Command, Snapshot},
    settings::Settings,
};
use design::*;
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use porch_core::SendState;
use porch_protocol::*;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

const PAGE: u32 = 0x14100D;
const SURFACE: u32 = 0x120E0B;
const RAISED: u32 = 0x1D1713;
const CREAM: u32 = 0xEDE8E4;
const MUTED: u32 = 0x8D827A;
const AMBER: u32 = 0xE6731E;
const BORDER: u32 = 0x221B16;
const SELECTED: u32 = 0x261E1A;
const OWN_BUBBLE: u32 = 0x4D2E16;
const OTHER_BUBBLE: u32 = 0x221912;
pub struct Porch {
    bridge: Bridge,
    snapshot: Arc<Snapshot>,
    root: PathBuf,
    settings: Settings,
    origin: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    invite: Entity<InputState>,
    composer: Entity<InputState>,
    search: Entity<InputState>,
    home: bool,
    people_view: bool,
    placeholder_for: Option<i64>,
    subscriptions: Vec<Subscription>,
    community: Option<i64>,
    show_settings: bool,
    register: bool,
    logout_confirm: bool,
    error: Option<String>,
    reply: Option<i64>,
    editing: Option<i64>,
    next_send: u64,
    awaiting: Option<(u64, i64, String, bool)>,
    drafts: HashMap<i64, String>,
    collapsed: HashSet<(i64, i64)>,
    message_scroll: ScrollHandle,
}
fn small(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text.into())
}
fn heading(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}
impl Porch {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::load(&root);
        let origin = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Server address")
                .default_value(settings.origin.clone())
        });
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let invite = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Platform invitation code")
                .masked(true)
        });
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Message")
                .auto_grow(1, 6)
        });
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find or start a conversation"));
        let bridge = Bridge::start(root.clone());
        let snapshot = bridge.latest.lock().unwrap().clone();
        let changes = bridge.changes.clone();
        cx.spawn_in(window, async move |this, cx| {
            while changes.recv().await.is_ok() {
                if this
                    .update_in(cx, |this, window, cx| this.receive(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let mut subscriptions =
            vec![
                cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { secondary: false }) {
                        this.send(window, cx)
                    }
                }),
            ];
        subscriptions.push(cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() && this.snapshot.voice.ptt {
                this.ptt(false, cx);
            }
        }));
        Self {
            bridge,
            snapshot,
            root,
            settings,
            origin,
            username,
            password,
            invite,
            composer,
            search,
            home: true,
            people_view: true,
            placeholder_for: None,
            subscriptions,
            community: None,
            show_settings: false,
            register: false,
            logout_confirm: false,
            error: None,
            reply: None,
            editing: None,
            next_send: 1,
            awaiting: None,
            drafts: HashMap::new(),
            collapsed: HashSet::new(),
            message_scroll: ScrollHandle::new(),
        }
    }
    fn receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.bridge.latest.lock().unwrap().clone();
        if snapshot.generation != self.snapshot.generation {
            self.community = None;
            self.reply = None;
            self.editing = None;
            self.awaiting = None;
            self.drafts.clear();
            self.composer
                .update(cx, |s, cx| s.set_value("", window, cx));
        }
        if let Some((id, conversation, text, edit)) = &self.awaiting {
            if if *edit {
                snapshot.edited == Some(*id)
            } else {
                snapshot.queued == Some(*id)
            } {
                if snapshot.selected == Some(*conversation)
                    && self.composer.read(cx).value().as_str() == text
                {
                    self.composer
                        .update(cx, |s, cx| s.set_value("", window, cx));
                }
                if self.drafts.get(conversation) == Some(text) {
                    self.drafts.remove(conversation);
                }
                self.awaiting = None;
                self.reply = None;
                self.editing = None;
            } else if snapshot.error.is_some() && !snapshot.busy {
                self.awaiting = None;
            }
        }
        if snapshot.selected != self.snapshot.selected {
            if snapshot.generation == self.snapshot.generation
                && let Some(id) = self.snapshot.selected
            {
                self.drafts
                    .insert(id, self.composer.read(cx).value().to_string());
            }
            let text = snapshot
                .selected
                .and_then(|id| self.drafts.get(&id))
                .cloned()
                .unwrap_or_default();
            let placeholder = snapshot
                .heads
                .iter()
                .find(|h| Some(h.id) == snapshot.selected)
                .map(|h| {
                    if h.community_id.is_some() {
                        format!("Message #{}", h.name)
                    } else {
                        format!("Message {}", self.head_title(h))
                    }
                })
                .unwrap_or_else(|| "Message".into());
            self.composer.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.set_placeholder(placeholder, window, cx)
            });
            self.reply = None;
            self.editing = None;
            self.community = snapshot
                .heads
                .iter()
                .find(|h| Some(h.id) == snapshot.selected)
                .and_then(|h| h.community_id);
        }
        self.drafts
            .retain(|id, _| snapshot.heads.iter().any(|head| head.id == *id));
        if snapshot.selected != self.snapshot.selected {
            self.message_scroll.scroll_to_bottom();
        }
        self.snapshot = snapshot;
        cx.notify();
    }
    fn command(&mut self, command: Command, cx: &mut Context<Self>) -> bool {
        match &command {
            Command::VoiceFlags {
                muted,
                deafened,
                ptt,
                held,
            } => self
                .bridge
                .gate
                .allow(!muted && !deafened && (!ptt || *held)),
            Command::VoiceLeave | Command::VoiceJoin(_) | Command::Logout => {
                self.bridge.gate.allow(false)
            }
            _ => {}
        }
        match self.bridge.commands.try_send(command) {
            Ok(()) => {
                self.error = None;
                cx.notify();
                true
            }
            Err(_) => {
                self.bridge.gate.allow(false);
                self.error = Some("Too many pending actions. Try again in a moment.".into());
                cx.notify();
                false
            }
        }
    }
    fn login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.busy {
            return;
        }
        let origin = self.origin.read(cx).value().trim().to_string();
        self.settings.origin = origin.clone();
        if self.settings.save(&self.root).is_err() {
            self.error = Some("Use a clean HTTPS server origin, or loopback HTTP, without URL credentials or query parameters. Check local settings permissions.".into());
            cx.notify();
            return;
        }
        let command = Command::Login {
            origin,
            username: self.username.read(cx).value().to_string(),
            password: self.password.read(cx).value().to_string(),
            invite: self
                .register
                .then(|| self.invite.read(cx).value().to_string()),
        };
        if self.command(command, cx) {
            self.password
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.invite.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }
    fn send(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.awaiting.is_some() {
            return;
        }
        let Some(conversation) = self.snapshot.selected else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        if let Some(message) = self.editing {
            let id = self.next_send;
            self.next_send += 1;
            if self.command(
                Command::Edit {
                    id,
                    conversation,
                    message,
                    text: text.clone(),
                },
                cx,
            ) {
                self.awaiting = Some((id, conversation, text, true));
            }
            return;
        }
        let id = self.next_send;
        self.next_send += 1;
        if self.command(
            Command::Send {
                id,
                conversation,
                text: text.clone(),
                reply_to: self.reply,
            },
            cx,
        ) {
            self.awaiting = Some((id, conversation, text, false));
        }
    }
    fn persist(&mut self, cx: &mut Context<Self>) {
        if self.settings.save(&self.root).is_err() {
            self.error = Some("Could not save local display settings.".into())
        }
        cx.notify();
    }
    fn logo(&self) -> impl IntoElement {
        // The supplied icon is a decoded local embedded PNG, never a remote fetch.
        img(ImageSource::Image(Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../../assets/brand/icons/porch-128.png").to_vec(),
        ))))
        .size(px(46.))
        .rounded_lg()
    }
    fn login_view(&self, cx: &mut Context<Self>) -> Div {
        div().size_full().flex().items_center().justify_center().bg(rgb(PAGE))
            .child(div().w(px(460.)).p_10().flex().flex_col().gap_5().bg(rgb(SURFACE)).rounded_xl().border_1().border_color(rgb(BORDER))
                .child(self.logo()).child(heading("Porch").text_size(px(28.)))
                .child(heading(if self.register {"Create account"} else {"Sign in"}))
                .child(Input::new(&self.origin)).child(Input::new(&self.username)).child(Input::new(&self.password))
                .when(self.register,|d|d.child(Input::new(&self.invite)))
                .child(Button::new("login").primary().label(if self.register{"Create account"}else{"Sign in"}).loading(self.snapshot.busy).on_click(cx.listener(|this,_,window,cx|this.login(window,cx))))
                .child(Button::new("register").ghost().label(if self.register{"I already have an account"}else{"Have a platform invitation?"}).on_click(cx.listener(|this,_,_,cx|{this.register= !this.register;cx.notify()})))
                .when(self.snapshot.notice.is_some(),|d|d.child(small("Local drafts were removed when your previous session ended. Some may already have been delivered. Sign in to inspect the notice."))))
    }
    fn pinned(&self, key: &str) -> bool {
        self.settings.pins.iter().any(|p| p == key)
    }
    fn toggle_pin(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(i) = self.settings.pins.iter().position(|p| *p == key) {
            self.settings.pins.remove(i);
        } else {
            self.settings.pins.push(key);
        }
        self.persist(cx);
        cx.notify();
    }
    fn open_community(&mut self, id: i64, cx: &mut Context<Self>) {
        self.home = false;
        self.community = Some(id);
        if let Some(head) = self
            .snapshot
            .heads
            .iter()
            .find(|h| h.community_id == Some(id))
        {
            self.command(Command::Select(head.id), cx);
        }
        cx.notify();
    }
    fn open_person(&mut self, id: i64, cx: &mut Context<Self>) {
        self.home = true;
        self.people_view = false;
        self.community = None;
        let me = self
            .snapshot
            .user
            .as_ref()
            .map(|u| u.id)
            .unwrap_or_default();
        let existing = self.snapshot.heads.iter().find(|h| {
            h.community_id.is_none()
                && h.participants.contains(&id)
                && h.participants.iter().all(|p| *p == id || *p == me)
        });
        match existing {
            Some(head) => {
                let hid = head.id;
                self.command(Command::Select(hid), cx);
            }
            None => {
                self.command(Command::CreateDm(vec![id]), cx);
            }
        }
        cx.notify();
    }
    fn people(&self) -> Vec<(i64, String, Vec<String>)> {
        let me = self
            .snapshot
            .user
            .as_ref()
            .map(|u| u.id)
            .unwrap_or_default();
        let mut out: Vec<(i64, String, Vec<String>)> = Vec::new();
        for c in &self.snapshot.communities {
            for (id, name) in &c.usernames {
                if *id == me {
                    continue;
                }
                match out.iter_mut().find(|p| p.0 == *id) {
                    Some(p) => p.2.push(c.community.name.clone()),
                    None => out.push((*id, name.clone(), vec![c.community.name.clone()])),
                }
            }
        }
        out.sort_by_key(|a| a.1.to_lowercase());
        out
    }
    fn topbar(&self, cx: &mut Context<Self>) -> Div {
        let busy = self.awaiting.is_some() || self.editing.is_some();
        let mut tabs = div().flex().items_center().gap_1().h_full();
        let mut keys: Vec<String> = self.settings.pins.clone();
        if keys.is_empty() {
            keys = self
                .snapshot
                .communities
                .iter()
                .map(|c| format!("c:{}", c.community.id))
                .collect();
        }
        for key in keys {
            let Some((kind, id)) = key
                .split_once(':')
                .and_then(|(k, i)| Some((k.to_string(), i.parse::<i64>().ok()?)))
            else {
                continue;
            };
            if kind == "c" {
                let Some(c) = self
                    .snapshot
                    .communities
                    .iter()
                    .find(|c| c.community.id == id)
                else {
                    continue;
                };
                let active = !self.home && self.community == Some(id);
                let unread = self
                    .snapshot
                    .heads
                    .iter()
                    .any(|h| h.community_id == Some(id) && h.unread_count > 0);
                tabs = tabs.child(
                    self.tab(
                        format!("tab-{key}"),
                        c.community.name.clone(),
                        id,
                        true,
                        active,
                        unread,
                    )
                    .when(!busy, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| this.open_community(id, cx)))
                    }),
                );
            } else {
                let name = self.member_name(id);
                let unread = self.snapshot.heads.iter().any(|h| {
                    h.community_id.is_none() && h.participants.contains(&id) && h.unread_count > 0
                });
                let active = self.home
                    && self.snapshot.heads.iter().any(|h| {
                        Some(h.id) == self.snapshot.selected
                            && h.community_id.is_none()
                            && h.participants.contains(&id)
                    });
                tabs = tabs.child(
                    self.tab(format!("tab-{key}"), name, id, false, active, unread)
                        .when(!busy, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| this.open_person(id, cx)))
                        }),
                );
            }
        }
        let faces = self.faces();
        div()
            .h(px(56.))
            .flex_shrink_0()
            .px_5()
            .flex()
            .items_center()
            .gap_5()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .child(
                div()
                    .id("home-wordmark")
                    .cursor_pointer()
                    .text_size(px(22.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if self.home { CREAM } else { MUTED }))
                    .child("Porch")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !busy {
                            this.home = true;
                            this.people_view = true;
                            this.community = None;
                            cx.notify();
                        }
                    })),
            )
            .child(tabs)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(faces.iter().map(|(id, name)| avatar(*id, name, 28.))),
            )
            .child(
                Button::new("settings")
                    .icon(icon("settings"))
                    .ghost()
                    .size(px(36.))
                    .tooltip("Display and session settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_settings = !this.show_settings;
                        cx.notify()
                    })),
            )
    }
    fn faces(&self) -> Vec<(i64, String)> {
        let mut out = Vec::new();
        if let Some(c) = self
            .snapshot
            .communities
            .iter()
            .find(|c| !self.home && Some(c.community.id) == self.community)
        {
            let mut v: Vec<_> = c.usernames.iter().collect();
            v.sort();
            out.extend(v.into_iter().take(5).map(|(i, n)| (*i, n.clone())));
        } else {
            out.extend(self.people().into_iter().take(5).map(|p| (p.0, p.1)));
        }
        out
    }
    fn tab(
        &self,
        id: String,
        name: String,
        ident: i64,
        group: bool,
        active: bool,
        unread: bool,
    ) -> Stateful<Div> {
        div()
            .id(SharedString::from(id))
            .h_full()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .border_b_2()
            .border_color(rgb(if active { AMBER } else { SURFACE }))
            .text_size(px(14.))
            .text_color(rgb(if active { CREAM } else { MUTED }))
            .child(if group {
                div()
                    .size(px(24.))
                    .rounded(px(7.))
                    .bg(rgb(0x435846))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(CREAM))
                    .child(
                        name.chars()
                            .next()
                            .map(|c| c.to_uppercase().to_string())
                            .unwrap_or_default(),
                    )
            } else {
                avatar(ident, &name, 24.)
            })
            .child(name)
            .when(unread, |d| {
                d.child(div().size(px(6.)).rounded_full().bg(rgb(AMBER)))
            })
    }
    fn ptt(&mut self, held: bool, cx: &mut Context<Self>) {
        let v = &self.snapshot.voice;
        if v.channel.is_some() && v.ptt {
            self.command(
                Command::VoiceFlags {
                    muted: v.manual_muted,
                    deafened: v.flags.deafened,
                    ptt: true,
                    held,
                },
                cx,
            );
        }
    }
    fn voice_shelf(&self, cx: &mut Context<Self>) -> Div {
        let v = &self.snapshot.voice;
        let mut d = div()
            .px_3()
            .py_3()
            .bg(rgb(RAISED))
            .flex()
            .flex_col()
            .gap_2();
        if v.channel.is_none() {
            return div();
        }
        let muted = v.manual_muted;
        let deafened = v.flags.deafened;
        let ptt = v.ptt;
        d = d
            .child(heading("Voice connected"))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new("voice-mute")
                            .xsmall()
                            .icon(icon(if muted { "mic-off" } else { "mic" }))
                            .tooltip(if muted { "Unmute" } else { "Mute" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.command(
                                    Command::VoiceFlags {
                                        muted: !muted,
                                        deafened,
                                        ptt,
                                        held: false,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("voice-deafen")
                            .xsmall()
                            .icon(icon("headphones"))
                            .tooltip(if deafened { "Undeafen" } else { "Deafen" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.command(
                                    Command::VoiceFlags {
                                        muted,
                                        deafened: !deafened,
                                        ptt,
                                        held: false,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("voice-leave")
                            .xsmall()
                            .icon(icon("leave"))
                            .tooltip("Disconnect")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.command(Command::VoiceLeave, cx);
                            })),
                    ),
            )
            .child(
                Button::new("voice-ptt")
                    .xsmall()
                    .selected(ptt)
                    .label(if ptt {
                        "Hold F9 to talk · window focused"
                    } else {
                        "Enable push-to-talk (F9)"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.command(
                            Command::VoiceFlags {
                                muted,
                                deafened,
                                ptt: !ptt,
                                held: false,
                            },
                            cx,
                        );
                    })),
            )
            .children(v.participants.iter().map(|p| {
                small(format!(
                    "{} Member {}{}",
                    if p.flags.speaking { "●" } else { "○" },
                    p.user_id,
                    if p.flags.deafened {
                        " · deafened"
                    } else if p.flags.muted {
                        " · muted"
                    } else {
                        ""
                    }
                ))
            }));
        for (input, names, index) in [(true, &v.inputs, v.input), (false, &v.outputs, v.output)] {
            let n = names.len();
            let label = format!(
                "{} · {}",
                if input { "Mic" } else { "Output" },
                names
                    .get(index)
                    .map(String::as_str)
                    .unwrap_or("System default")
            );
            d = d.child(
                Button::new(if input { "voice-input" } else { "voice-output" })
                    .xsmall()
                    .label(label)
                    .disabled(n == 0)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if n > 0 {
                            this.command(
                                Command::VoiceDevice {
                                    input,
                                    index: (index + 1) % n,
                                },
                                cx,
                            );
                        }
                    })),
            );
        }
        d
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let community = self
            .snapshot
            .communities
            .iter()
            .find(|c| Some(c.community.id) == self.community);
        let mut list = div()
            .id("channels")
            .flex_1()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .flex()
            .flex_col()
            .gap_1();
        if let Some(c) = community {
            for category in std::iter::once(None).chain(c.categories.iter().map(Some)) {
                if let Some(category) = category {
                    let key = (c.community.id, category.id);
                    let collapsed = self.collapsed.contains(&key);
                    list = list.child(
                        Button::new(SharedString::from(format!("category-{}-{}", key.0, key.1)))
                            .ghost()
                            .xsmall()
                            .w_full()
                            .justify_start()
                            .mt_4()
                            .icon(icon(if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            }))
                            .label(category.name.to_uppercase())
                            .text_color(rgb(MUTED))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.collapsed.remove(&key) {
                                    this.collapsed.insert(key);
                                }
                                cx.notify();
                            })),
                    );
                    if collapsed {
                        continue;
                    }
                }
                let cat = category.map(|c| c.id);
                for channel in c.channels.iter().filter(|ch| ch.category_id == cat) {
                    let head = self.snapshot.heads.iter().find(|h| {
                        h.community_id == Some(c.community.id) && h.channel_id == Some(channel.id)
                    });
                    if channel.kind == ChannelKind::Voice {
                        let ch = VoiceChannel {
                            community_id: c.community.id,
                            channel_id: channel.id,
                        };
                        list = list.child(
                            Button::new(SharedString::from(format!(
                                "voice-{}-{}",
                                ch.community_id, ch.channel_id
                            )))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .icon(icon("volume"))
                            .label(channel.name.clone())
                            .selected(self.snapshot.voice.channel == Some(ch))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.command(Command::VoiceJoin(ch), cx);
                                },
                            )),
                        );
                        if self.snapshot.voice.channel == Some(ch) {
                            for participant in &self.snapshot.voice.participants {
                                list = list.child(
                                    div()
                                        .ml_5()
                                        .py_1()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(avatar(
                                            participant.user_id,
                                            &self.member_name(participant.user_id),
                                            24.,
                                        ))
                                        .child(small(self.member_name(participant.user_id)))
                                        .when(participant.flags.speaking, |d| {
                                            d.child(icon("volume").text_color(rgb(0x91BC99)))
                                        }),
                                );
                            }
                        }
                        continue;
                    }
                    if let Some(head) = head {
                        list = list.child(self.channel_button(head, cx));
                    }
                }
            }
        } else {
            let q = self.search.read(cx).value().to_lowercase();
            let mut heads: Vec<&ConversationHead> = self
                .snapshot
                .heads
                .iter()
                .filter(|h| {
                    h.community_id.is_none() && self.head_title(h).to_lowercase().contains(q.trim())
                })
                .collect();
            heads.sort_by_key(|a| std::cmp::Reverse(a.last_message_id));
            for head in heads {
                list = list.child(self.dm_row(head, cx));
            }
        }
        let header = if let Some(c) = community {
            let key = format!("c:{}", c.community.id);
            let pinned = self.pinned(&key);
            div()
                .h(px(48.))
                .px_4()
                .flex()
                .items_center()
                .justify_between()
                .child(heading(c.community.name.clone()).text_size(px(16.)))
                .child(
                    Button::new("pin-community")
                        .ghost()
                        .xsmall()
                        .icon(icon("pin"))
                        .selected(pinned)
                        .tooltip(if pinned {
                            "Unpin from top bar"
                        } else {
                            "Pin to top bar"
                        })
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_pin(key.clone(), cx)),
                        ),
                )
        } else {
            div().px_3().pt_4().pb_2().flex().flex_col().gap_2().child(
                div()
                    .h(px(40.))
                    .rounded(px(8.))
                    .bg(rgb(RAISED))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("search").text_color(rgb(MUTED)))
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.search).appearance(false).bordered(false)),
                    ),
            )
        };
        div()
            .w(px(if community.is_some() { 260. } else { 340. }))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(SURFACE))
            .border_r_1()
            .border_color(rgb(BORDER))
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .child(self.voice_shelf(cx))
            .child(self.user_panel(cx))
    }
    fn head_title(&self, head: &ConversationHead) -> String {
        if head.community_id.is_some() {
            return head.name.clone();
        }
        let me = self
            .snapshot
            .user
            .as_ref()
            .map(|u| u.id)
            .unwrap_or_default();
        let names: Vec<String> = head
            .participants
            .iter()
            .filter(|p| **p != me)
            .map(|p| self.member_name(*p))
            .collect();
        if names.is_empty() || names.iter().any(|n| n.starts_with("User ")) {
            head.name.clone()
        } else {
            names.join(", ")
        }
    }
    fn dm_row(&self, head: &ConversationHead, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = head.id;
        let me = self
            .snapshot
            .user
            .as_ref()
            .map(|u| u.id)
            .unwrap_or_default();
        let other = head
            .participants
            .iter()
            .copied()
            .find(|p| *p != me)
            .unwrap_or(id);
        let selected = self.snapshot.selected == Some(id) && !self.people_view;
        let busy = self.awaiting.is_some() || self.editing.is_some();
        div()
            .id(SharedString::from(format!("channel-{id}")))
            .h(px(60.))
            .px_3()
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap_3()
            .cursor_pointer()
            .bg(rgb(if selected { SELECTED } else { SURFACE }))
            .hover(|d| d.bg(rgb(RAISED)))
            .child(avatar(other, &self.head_title(head), 36.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(heading(self.head_title(head)).text_size(px(14.)))
                    .child(
                        small(if head.unread_count > 0 {
                            format!("{} unread", head.unread_count)
                        } else {
                            String::new()
                        })
                        .text_size(px(13.)),
                    ),
            )
            .when(head.unread_count > 0, |d| {
                d.child(
                    div()
                        .min_w(px(18.))
                        .h(px(18.))
                        .rounded_full()
                        .bg(rgb(AMBER))
                        .text_size(px(11.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(head.unread_count.to_string()),
                )
            })
            .when(!busy, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.people_view = false;
                    this.command(Command::Select(id), cx);
                    cx.notify();
                }))
            })
    }
    fn people_panel(&self, cx: &mut Context<Self>) -> Div {
        let people = self.people();
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(PAGE))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(56.))
                    .px_6()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(heading("People").text_size(px(16.)))
                    .child(small(people.len().to_string())),
            )
            .child(
                div()
                    .id("people")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_6()
                    .children(people.into_iter().map(|(id, name, groups)| {
                        let key = format!("u:{id}");
                        let pinned = self.pinned(&key);
                        div()
                            .h(px(62.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_3()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(avatar(id, &name, 40.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(heading(name).text_size(px(15.)))
                                    .child(small(groups.join(", "))),
                            )
                            .child(
                                Button::new(SharedString::from(format!("pin-{id}")))
                                    .ghost()
                                    .small()
                                    .icon(icon("pin"))
                                    .selected(pinned)
                                    .tooltip(if pinned {
                                        "Unpin from top bar"
                                    } else {
                                        "Pin to top bar"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_pin(key.clone(), cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("msg-{id}")))
                                    .ghost()
                                    .small()
                                    .icon(icon("send"))
                                    .tooltip("Message")
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.open_person(id, cx)),
                                    ),
                            )
                    })),
            )
    }
    fn channel_button(&self, head: &ConversationHead, cx: &mut Context<Self>) -> Button {
        let id = head.id;
        let label = if head.unread_count > 0 {
            format!("{}   {}", head.name, head.unread_count)
        } else {
            head.name.clone()
        };
        Button::new(SharedString::from(format!("channel-{id}")))
            .ghost()
            .icon(icon(if head.community_id.is_some() {
                "hash"
            } else {
                "user"
            }))
            .label(label)
            .selected(self.snapshot.selected == Some(id))
            .disabled(self.awaiting.is_some() || self.editing.is_some())
            .justify_start()
            .w_full()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.people_view = false;
                this.command(Command::Select(id), cx);
                cx.notify();
            }))
    }
    fn message(&self, message: &Message, grouped: bool, cx: &mut Context<Self>) -> Div {
        let id = message.id;
        let conversation = message.conversation_id;
        let author = self.member_name(message.author_id);
        let text = match &message.content {
            Some(MessageContent::Markdown { text }) => text.clone(),
            _ => "[message removed]".into(),
        };
        let own = self
            .snapshot
            .user
            .as_ref()
            .is_some_and(|u| u.id == message.author_id);
        let dm = self
            .snapshot
            .heads
            .iter()
            .any(|h| h.id == conversation && h.community_id.is_none());
        let mut row = div()
            .flex_shrink_0()
            .group("message")
            .relative()
            .px_3()
            .py_1()
            .mt(if grouped { px(0.) } else { px(8.) })
            .hover(|d| d.bg(rgb(0x181310)))
            .flex()
            .gap_3()
            .child(if grouped {
                div().w(px(38.)).flex_shrink_0()
            } else {
                avatar(message.author_id, &author, 38.)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(!grouped, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(heading(author).text_size(px(14.)))
                                .child(small(clock(&message.created_at)).text_size(px(11.)))
                                .when(message.edited_at.is_some(), |d| d.child(small("edited")))
                                .when(message.pinned, |d| d.child(icon("pin"))),
                        )
                    })
                    .when(message.reply_to.is_some(), |d| {
                        d.child(small(format!(
                            "Reply to message #{}",
                            message.reply_to.unwrap_or_default()
                        )))
                    })
                    .child(if dm {
                        div().flex().child(
                            div()
                                .max_w(px(640.))
                                .px_4()
                                .py_2()
                                .rounded(px(14.))
                                .bg(rgb(if own { OWN_BUBBLE } else { OTHER_BUBBLE }))
                                .whitespace_normal()
                                .child(text.clone()),
                        )
                    } else {
                        div().whitespace_normal().child(text.clone())
                    })
                    .child(self.attachments(message, cx))
                    .when(!message.reactions.is_empty(), |d| {
                        d.child(small(
                            message
                                .reactions
                                .iter()
                                .map(|r| format!("{} {}", r.emoji, r.users.len()))
                                .collect::<Vec<_>>()
                                .join("  "),
                        ))
                    }),
            );
        if !message.deleted {
            row = row.child(
                div()
                    .flex()
                    .absolute()
                    .right_4()
                    .top(px(-12.))
                    .bg(rgb(RAISED))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .invisible()
                    .group_hover("message", |d| d.visible())
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("reply-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(icon("reply"))
                            .tooltip("Reply")
                            .disabled(self.awaiting.is_some() || self.editing.is_some())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.reply = Some(id);
                                this.editing = None;
                                this.composer.update(cx, |s, cx| s.focus(window, cx));
                                cx.notify()
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("react-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(icon("heart"))
                            .tooltip("Toggle a thumbs-up reaction")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let active = !this
                                    .snapshot
                                    .messages
                                    .iter()
                                    .find(|m| m.id == id)
                                    .is_some_and(|m| {
                                        m.reactions.iter().any(|r| {
                                            r.emoji == "👍"
                                                && this
                                                    .snapshot
                                                    .user
                                                    .as_ref()
                                                    .is_some_and(|u| r.users.contains(&u.id))
                                        })
                                    });
                                this.command(
                                    Command::Mutate {
                                        conversation,
                                        message: id,
                                        command: MessageCommand::React {
                                            emoji: "👍".into(),
                                            active,
                                        },
                                    },
                                    cx,
                                );
                            })),
                    )
                    .when(own, |d| {
                        d.child(
                            Button::new(SharedString::from(format!("edit-{id}")))
                                .ghost()
                                .xsmall()
                                .icon(icon("edit"))
                                .tooltip("Edit message")
                                .disabled(self.awaiting.is_some() || self.editing.is_some())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.editing = Some(id);
                                    this.reply = None;
                                    this.composer
                                        .update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                                    cx.notify()
                                })),
                        )
                    }),
            );
        }
        row
    }
    fn conversation(&self, cx: &mut Context<Self>) -> Div {
        let head = self
            .snapshot
            .heads
            .iter()
            .find(|h| Some(h.id) == self.snapshot.selected);
        let mut panel = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(PAGE))
            .flex()
            .flex_col();
        let Some(head) = head else {
            return panel
                .items_center()
                .justify_center()
                .gap_4()
                .child(self.logo())
                .child(heading("Select a conversation"));
        };
        panel = panel
            .child(
                div()
                    .h(px(56.))
                    .flex_shrink_0()
                    .px_6()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(if head.community_id.is_some() {
                        icon("hash").text_color(rgb(MUTED)).into_any_element()
                    } else {
                        avatar(
                            head.participants
                                .iter()
                                .copied()
                                .find(|p| Some(*p) != self.snapshot.user.as_ref().map(|u| u.id))
                                .unwrap_or(head.id),
                            &self.head_title(head),
                            34.,
                        )
                        .into_any_element()
                    })
                    .child(heading(self.head_title(head)).text_size(px(16.))),
            )
            .child(
                div()
                    .id(("messages", head.id as u64))
                    .track_scroll(&self.message_scroll)
                    .pb_5()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .p_5()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("older")
                                    .outline()
                                    .small()
                                    .label("Load earlier messages")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.command(Command::Older, cx);
                                    })),
                            )
                            .child(
                                Button::new("latest")
                                    .disabled(self.awaiting.is_some() || self.editing.is_some())
                                    .outline()
                                    .small()
                                    .label("Latest")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.snapshot.selected {
                                            this.command(Command::Select(id), cx);
                                        }
                                    })),
                            ),
                    )
                    .when(self.snapshot.messages.is_empty(), |d| {
                        d.child(
                            div()
                                .px_6()
                                .py_5()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(small("No messages yet.")),
                        )
                    })
                    .children(self.snapshot.messages.iter().enumerate().map(|(i, m)| {
                        let grouped = i
                            .checked_sub(1)
                            .and_then(|n| self.snapshot.messages.get(n))
                            .is_some_and(|previous| grouped(previous, m));
                        self.message(m, grouped, cx)
                    }))
                    .children(
                        self.snapshot
                            .pending
                            .iter()
                            .filter(|p| p.conversation_id == head.id)
                            .map(|p| {
                                let nonce = p.request.nonce.clone();
                                let discard = nonce.clone();
                                let failed = p.state == SendState::Failed;
                                div()
                                    .p_5()
                                    .bg(rgb(SURFACE))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(small(if failed {
                                        "Not sent · rejected by server"
                                    } else {
                                        "Queued · waiting for confirmation"
                                    }))
                                    .child(match &p.request.content {
                                        MessageContent::Markdown { text } => text.clone(),
                                        _ => String::new(),
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .when(failed, |d| {
                                                d.child(
                                                    Button::new(SharedString::from(format!(
                                                        "retry-{nonce}"
                                                    )))
                                                    .small()
                                                    .label("Retry")
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.command(
                                                            Command::Retry(nonce.clone()),
                                                            cx,
                                                        );
                                                    })),
                                                )
                                            })
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "discard-{discard}"
                                                )))
                                                .ghost()
                                                .small()
                                                .label("Discard local copy")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.command(
                                                        Command::Discard(discard.clone()),
                                                        cx,
                                                    );
                                                })),
                                            ),
                                    )
                                    .child(small(
                                        "Discarding cannot retract a message already delivered.",
                                    ))
                            }),
                    ),
            )
            .child(
                div()
                    .px_6()
                    .pb_5()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when(self.reply.is_some() || self.editing.is_some(), |d| {
                        d.child(
                            div()
                                .flex()
                                .gap_2()
                                .child(small(format!(
                                    "{} #{}",
                                    if self.editing.is_some() {
                                        "Editing"
                                    } else {
                                        "Replying to"
                                    },
                                    self.editing.or(self.reply).unwrap_or_default()
                                )))
                                .child(
                                    Button::new("cancel-reply")
                                        .ghost()
                                        .xsmall()
                                        .label("Cancel")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reply = None;
                                            this.editing = None;
                                            cx.notify()
                                        })),
                                ),
                        )
                    })
                    .child(self.transfers(cx))
                    .child(
                        Input::new(&self.composer)
                            .prefix(
                                Button::new("choose-file")
                                    .ghost()
                                    .icon(icon("paperclip"))
                                    .tooltip("Attach a file")
                                    .disabled(self.editing.is_some() || self.snapshot.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_file(window, cx)
                                    })),
                            )
                            .suffix(
                                Button::new("send")
                                    .primary()
                                    .rounded_full()
                                    .bg(rgb(AMBER))
                                    .icon(icon("send"))
                                    .tooltip(if self.editing.is_some() {
                                        "Save edit"
                                    } else {
                                        "Send message"
                                    })
                                    .disabled(self.awaiting.is_some())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.send(window, cx)),
                                    ),
                            ),
                    ),
            );
        panel
    }
    fn settings_panel(&self, cx: &mut Context<Self>) -> Div {
        div().p_5().bg(rgb(RAISED)).flex().flex_col().gap_3().border_b_1().border_color(rgb(BORDER))
            .child(heading("Settings"))
            .child(div().flex().items_center().gap_3()
                .child(small(format!("Message size · {}px",self.settings.font_size)))
                .child(Button::new("smaller").label("A−").on_click(cx.listener(|this,_,_,cx|{this.settings.font_size=(this.settings.font_size-1.).max(12.);this.persist(cx)})))
                .child(Button::new("larger").label("A+").on_click(cx.listener(|this,_,_,cx|{this.settings.font_size=(this.settings.font_size+1.).min(22.);this.persist(cx)})))
                .child(Button::new("toggle-members").label(if self.settings.members{"Hide members"}else{"Show members"}).on_click(cx.listener(|this,_,_,cx|{this.settings.members= !this.settings.members;this.persist(cx)})))
                .child(Button::new("sync").label("Reconnect / sync").on_click(cx.listener(|this,_,_,cx|{this.command(Command::Refresh,cx);})))
                .child(Button::new("logout").label(if self.logout_confirm{"Confirm sign out"}else{"Sign out"}).on_click(cx.listener(|this,_,_,cx|{
                    if this.logout_confirm{this.command(Command::Logout,cx);this.logout_confirm=false;}
                    else{this.logout_confirm=true;cx.notify();}
                }))))
            .child(small(format!("{} local queued drafts. Signing out removes local history and drafts. Delivery may already have succeeded.",self.snapshot.pending.len())))
            .child(small(format!("Local data · {} · readable SQLite, not encrypted",self.root.display())))
    }
}
impl Render for Porch {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep the subscriptions alive for the view lifetime.
        let _ = &self.subscriptions;
        if self.placeholder_for != self.snapshot.selected {
            self.placeholder_for = self.snapshot.selected;
            let text = self
                .snapshot
                .heads
                .iter()
                .find(|h| Some(h.id) == self.snapshot.selected)
                .map(|h| {
                    if h.community_id.is_some() {
                        format!("Message #{}", h.name)
                    } else {
                        format!("Message {}", self.head_title(h))
                    }
                })
                .unwrap_or_else(|| "Message".into());
            self.composer
                .update(cx, |s, cx| s.set_placeholder(text, _window, cx));
        }
        div().size_full().flex().flex_col()
            .on_key_down(cx.listener(|this,e:&KeyDownEvent,_,cx|{if e.keystroke.key=="f9"&&!e.is_held{this.ptt(true,cx);}}))
            .on_key_up(cx.listener(|this,e:&KeyUpEvent,_,cx|{if e.keystroke.key=="f9"{this.ptt(false,cx);}}))
            .font_family("Manrope").font_weight(FontWeight::MEDIUM).text_size(px(self.settings.font_size)).text_color(rgb(CREAM)).bg(rgb(PAGE))
            .when(self.snapshot.user.is_some()&&self.show_settings,|d|d.child(self.settings_panel(cx)))
            .when_some(self.error.clone().or_else(||self.snapshot.error.clone()),|d,e|d.child(div().px_5().py_3().bg(rgb(0x45271F)).child(e)))
            .when(self.snapshot.user.is_some()&&self.snapshot.notice.is_some(),|d|d.child(div().p_3().bg(rgb(RAISED)).flex().gap_3()
                .child(small(format!("{} local drafts were removed when access ended. Some may already have been delivered.",self.snapshot.notice.as_ref().map(|n|n.sends.len()).unwrap_or_default())))
                .child(Button::new("ack-notice").ghost().small().label("Dismiss").on_click(cx.listener(|this,_,_,cx|{this.command(Command::AcknowledgeNotice,cx);})))))
            .child(if self.snapshot.user.is_none(){self.login_view(cx).into_any_element()}else{
                div().flex_1().min_h_0().flex().flex_col().child(self.topbar(cx)).child(div().flex_1().min_h_0().flex().child(self.sidebar(cx)).child(if self.home && self.people_view { self.people_panel(cx) } else { self.conversation(cx) })).into_any_element()
            })
    }
}
