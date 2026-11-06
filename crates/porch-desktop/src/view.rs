use crate::{
    session::{Bridge, Command, Snapshot},
    settings::Settings,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use porch_core::SendState;
use porch_protocol::*;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

const PAGE: u32 = 0x111210;
const SURFACE: u32 = 0x191D19;
const RAISED: u32 = 0x232B24;
const CREAM: u32 = 0xF4E8D7;
const MUTED: u32 = 0xB8B5A6;
const AMBER: u32 = 0xF3B85A;
const BORDER: u32 = 0x354136;
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
    dm: Entity<InputState>,
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
}
fn small(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text.into())
}
fn heading(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::BOLD).child(text.into())
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
                .placeholder("Pull up. Say something.")
                .auto_grow(2, 6)
        });
        let dm =
            cx.new(|cx| InputState::new(window, cx).placeholder("Member IDs, separated by commas"));
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
        let subscriptions =
            vec![
                cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { secondary: false }) {
                        this.send(window, cx)
                    }
                }),
            ];
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
            dm,
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
            self.composer
                .update(cx, |s, cx| s.set_value(text, window, cx));
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
        self.snapshot = snapshot;
        cx.notify();
    }
    fn command(&mut self, command: Command, cx: &mut Context<Self>) -> bool {
        match self.bridge.commands.try_send(command) {
            Ok(()) => {
                self.error = None;
                cx.notify();
                true
            }
            Err(_) => {
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
                .child(self.logo()).child(div().text_size(px(36.)).font_weight(FontWeight::BOLD).child("Your people."))
                .child(div().text_size(px(36.)).font_weight(FontWeight::BOLD).mt(px(-12.)).text_color(rgb(AMBER)).child("Your place."))
                .child(small("A private place to pull up and talk awhile."))
                .child(Input::new(&self.origin)).child(Input::new(&self.username)).child(Input::new(&self.password))
                .when(self.register,|d|d.child(Input::new(&self.invite)))
                .child(Button::new("login").primary().label(if self.register{"Create account"}else{"Sign in"}).loading(self.snapshot.busy).on_click(cx.listener(|this,_,window,cx|this.login(window,cx))))
                .child(Button::new("register").ghost().label(if self.register{"I already have an account"}else{"Have a platform invitation?"}).on_click(cx.listener(|this,_,_,cx|{this.register= !this.register;cx.notify()})))
                .child(small("Your password and session are not saved on this device."))
                .when(self.snapshot.notice.is_some(),|d|d.child(small("Local drafts were removed when your previous session ended. Some may already have been delivered. Sign in to inspect the notice."))))
    }
    fn rail(&self, cx: &mut Context<Self>) -> Div {
        div()
            .w(px(76.))
            .flex_shrink_0()
            .h_full()
            .py_4()
            .px_3()
            .bg(rgb(PAGE))
            .flex()
            .flex_col()
            .gap_3()
            .child(self.logo())
            .child(
                Button::new("home")
                    .label("DM")
                    .tooltip("Direct messages")
                    .selected(self.community.is_none())
                    .disabled(self.awaiting.is_some() || self.editing.is_some())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.community = None;
                        if let Some(head) = this
                            .snapshot
                            .heads
                            .iter()
                            .find(|h| h.community_id.is_none())
                        {
                            this.command(Command::Select(head.id), cx);
                        }
                        cx.notify()
                    })),
            )
            .child(div().h(px(1.)).bg(rgb(BORDER)).my_1())
            .children(self.snapshot.communities.iter().map(|c| {
                let id = c.community.id;
                let name = c.community.name.clone();
                let initials: String = name
                    .split_whitespace()
                    .take(2)
                    .filter_map(|s| s.chars().next())
                    .collect();
                Button::new(SharedString::from(format!("community-{id}")))
                    .label(initials)
                    .tooltip(name)
                    .selected(self.community == Some(id))
                    .disabled(self.awaiting.is_some() || self.editing.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.community = Some(id);
                        if let Some(head) = this
                            .snapshot
                            .heads
                            .iter()
                            .find(|h| h.community_id == Some(id))
                        {
                            this.command(Command::Select(head.id), cx);
                        }
                        cx.notify()
                    }))
            }))
            .child(div().flex_1())
            .child(
                Button::new("settings")
                    .label("Aa")
                    .tooltip("Display and session settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_settings = !this.show_settings;
                        cx.notify()
                    })),
            )
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let community = self
            .snapshot
            .communities
            .iter()
            .find(|c| Some(c.community.id) == self.community);
        let title = community
            .map(|c| c.community.name.as_str())
            .unwrap_or("Your conversations");
        let mut list = div()
            .id("channels")
            .flex_1()
            .overflow_y_scroll()
            .px_3()
            .py_4()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(c) = community {
            for category in std::iter::once(None).chain(c.categories.iter().map(Some)) {
                if let Some(category) = category {
                    list = list.child(small(category.name.clone()).mt_4());
                }
                let cat = category.map(|c| c.id);
                for channel in c.channels.iter().filter(|ch| ch.category_id == cat) {
                    let head = self.snapshot.heads.iter().find(|h| {
                        h.community_id == Some(c.community.id) && h.channel_id == Some(channel.id)
                    });
                    if channel.kind == ChannelKind::Voice {
                        list = list.child(small(format!("◌ {} · voice later", channel.name)));
                        continue;
                    }
                    if let Some(head) = head {
                        list = list.child(self.channel_button(head, cx));
                    }
                }
            }
        } else {
            list = list.child(small("DIRECT MESSAGES"));
            for head in self
                .snapshot
                .heads
                .iter()
                .filter(|h| h.community_id.is_none())
            {
                list = list.child(self.channel_button(head, cx));
            }
            list = list.child(
                div()
                    .mt_5()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(small("START A CONVERSATION"))
                    .child(Input::new(&self.dm))
                    .child(
                        Button::new("new-dm")
                            .disabled(self.awaiting.is_some() || self.editing.is_some())
                            .label("Open DM")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let raw = this.dm.read(cx).value();
                                let ids: Result<Vec<i64>, _> =
                                    raw.split(',').map(|s| s.trim().parse()).collect();
                                match ids {
                                    Ok(ids)
                                        if !ids.is_empty()
                                            && ids.len() <= 19
                                            && ids.iter().all(|id| *id > 0) =>
                                    {
                                        this.command(Command::CreateDm(ids), cx);
                                    }
                                    _ => {
                                        this.error = Some(
                                            "Enter 1–19 positive member IDs, separated by commas."
                                                .into(),
                                        );
                                        cx.notify();
                                    }
                                }
                            })),
                    ),
            );
        }
        div()
            .w(px(254.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(SURFACE))
            .border_r_1()
            .border_color(rgb(BORDER))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(76.))
                    .px_5()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(heading(title.to_string())),
            )
            .child(list)
            .child(
                div()
                    .p_4()
                    .bg(rgb(RAISED))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(heading(
                        self.snapshot
                            .user
                            .as_ref()
                            .map(|u| u.username.clone())
                            .unwrap_or_default(),
                    ))
                    .child(small(format!(
                        "{} · member #{}",
                        self.snapshot.state,
                        self.snapshot
                            .user
                            .as_ref()
                            .map(|u| u.id)
                            .unwrap_or_default()
                    ))),
            )
    }
    fn channel_button(&self, head: &ConversationHead, cx: &mut Context<Self>) -> Button {
        let id = head.id;
        let label = if head.unread_count > 0 {
            format!("# {}   {}", head.name, head.unread_count)
        } else {
            format!("# {}", head.name)
        };
        Button::new(SharedString::from(format!("channel-{id}")))
            .ghost()
            .label(label)
            .selected(self.snapshot.selected == Some(id))
            .disabled(self.awaiting.is_some() || self.editing.is_some())
            .justify_start()
            .w_full()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.command(Command::Select(id), cx);
            }))
    }
    fn message(&self, message: &Message, cx: &mut Context<Self>) -> Div {
        let id = message.id;
        let conversation = message.conversation_id;
        let author = self
            .snapshot
            .user
            .as_ref()
            .filter(|u| u.id == message.author_id)
            .map(|u| u.username.clone())
            .unwrap_or_else(|| format!("Member #{}", message.author_id));
        let text = match &message.content {
            Some(MessageContent::Markdown { text }) => text.clone(),
            _ => "[message removed]".into(),
        };
        let own = self
            .snapshot
            .user
            .as_ref()
            .is_some_and(|u| u.id == message.author_id);
        let mut row = div()
            .px_6()
            .py_3()
            .flex()
            .gap_3()
            .border_b_1()
            .border_color(rgb(0x202720))
            .child(
                div()
                    .size(px(34.))
                    .flex_shrink_0()
                    .rounded_lg()
                    .bg(rgb(RAISED))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(AMBER))
                    .child(author.chars().next().unwrap_or('?').to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(heading(author))
                            .child(small(message.created_at.clone()))
                            .when(message.edited_at.is_some(), |d| d.child(small("edited")))
                            .when(message.pinned, |d| d.child(small("pinned"))),
                    )
                    .when(message.reply_to.is_some(), |d| {
                        d.child(small(format!(
                            "Reply to message #{}",
                            message.reply_to.unwrap_or_default()
                        )))
                    })
                    .child(div().whitespace_normal().child(text.clone()))
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
                    .flex_col()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("reply-{id}")))
                            .ghost()
                            .xsmall()
                            .label("Reply")
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
                            .label("+1")
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
                                .label("Edit")
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
                .child(heading("Pull up a chair."))
                .child(small(
                    "Choose a channel or a direct message to get started.",
                ));
        };
        panel = panel
            .child(
                div()
                    .h(px(76.))
                    .flex_shrink_0()
                    .px_6()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(heading(format!("# {}", head.name)))
                            .child(small("Your people. Your place.")),
                    ),
            )
            .child(
                div()
                    .id(("messages", head.id as u64))
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
                                    .ghost()
                                    .small()
                                    .label("Load earlier messages")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.command(Command::Older, cx);
                                    })),
                            )
                            .child(
                                Button::new("latest")
                                    .disabled(self.awaiting.is_some() || self.editing.is_some())
                                    .ghost()
                                    .small()
                                    .label("Latest")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.snapshot.selected {
                                            this.command(Command::Select(id), cx);
                                        }
                                    })),
                            )
                            .child(small("Plain Markdown · no remote images loaded")),
                    )
                    .children(self.snapshot.messages.iter().map(|m| self.message(m, cx)))
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
                    .p_5()
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
                    .child(Input::new(&self.composer))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(small("Enter to send · Shift+Enter for a new line"))
                            .child(
                                Button::new("send")
                                    .primary()
                                    .label(if self.editing.is_some() {
                                        "Save edit"
                                    } else {
                                        "Send"
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
    fn members(&self) -> impl IntoElement {
        let community = self
            .snapshot
            .communities
            .iter()
            .find(|c| Some(c.community.id) == self.community);
        let ids: Vec<_> = if let Some(c) = community {
            c.members.iter().map(|m| m.user_id).collect()
        } else {
            self.snapshot
                .heads
                .iter()
                .find(|h| Some(h.id) == self.snapshot.selected)
                .map(|h| h.participants.clone())
                .unwrap_or_default()
        };
        div()
            .id("members")
            .w(px(208.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(SURFACE))
            .border_l_1()
            .border_color(rgb(BORDER))
            .p_5()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .child(small(format!("MEMBERS · {}", ids.len())))
            .children(ids.into_iter().map(|id| {
                let name = self
                    .snapshot
                    .user
                    .as_ref()
                    .filter(|u| u.id == id)
                    .map(|u| format!("{} · you", u.username))
                    .unwrap_or_else(|| format!("Member #{id}"));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(8.)).rounded_full().bg(rgb(MUTED)))
                    .child(name)
            }))
            .child(small(
                "Membership, not live presence. Profiles arrive in a later phase.",
            ))
    }
    fn settings_panel(&self, cx: &mut Context<Self>) -> Div {
        div().p_5().bg(rgb(RAISED)).flex().flex_col().gap_3().border_b_1().border_color(rgb(BORDER))
            .child(heading("Make yourself comfortable."))
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
        div().size_full().flex().flex_col().font_family("Manrope").text_size(px(self.settings.font_size)).text_color(rgb(CREAM)).bg(rgb(PAGE))
            .when(self.snapshot.user.is_some()&&self.show_settings,|d|d.child(self.settings_panel(cx)))
            .when_some(self.error.clone().or_else(||self.snapshot.error.clone()),|d,e|d.child(div().px_5().py_3().bg(rgb(0x45271F)).child(e)))
            .when(self.snapshot.user.is_some()&&self.snapshot.notice.is_some(),|d|d.child(div().p_3().bg(rgb(RAISED)).flex().gap_3()
                .child(small(format!("{} local drafts were removed when access ended. Some may already have been delivered.",self.snapshot.notice.as_ref().map(|n|n.sends.len()).unwrap_or_default())))
                .child(Button::new("ack-notice").ghost().small().label("Dismiss").on_click(cx.listener(|this,_,_,cx|{this.command(Command::AcknowledgeNotice,cx);})))))
            .child(if self.snapshot.user.is_none(){self.login_view(cx).into_any_element()}else{
                div().flex_1().min_h_0().flex().child(self.rail(cx)).child(self.sidebar(cx)).child(self.conversation(cx)).when(self.settings.members,|d|d.child(self.members())).into_any_element()
            })
    }
}
