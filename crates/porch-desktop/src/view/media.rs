//! Media remains opt-in: choosing, transferring, sending, decoding and saving
//! are distinct actions. Never give GPUI a URL carrying bearer credentials.
use super::*;
impl Porch {
    pub(super) fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.snapshot.selected else {
            return;
        };
        let generation = self.snapshot.generation;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a file to stage (not uploaded yet)".into()),
        });
        cx.spawn_in(window, async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, _, cx| {
                        if this.snapshot.generation == generation
                            && this.snapshot.selected == Some(conversation)
                        {
                            this.command(Command::ChooseFile { conversation, path }, cx);
                        }
                    });
                }
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.error = Some("The file picker could not be opened.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn save_file(
        &mut self,
        conversation: i64,
        message: i64,
        file: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.snapshot.generation;
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |this, cx| match picker.await {
            Ok(Ok(Some(path))) => {
                let _ = this.update_in(cx, |this, _, cx| {
                    if this.snapshot.generation == generation {
                        this.command(
                            Command::Download {
                                conversation,
                                message,
                                file,
                                path,
                            },
                            cx,
                        );
                    }
                });
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.error = Some("The save dialog could not be opened.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub(super) fn attachments(&self, message: &Message, cx: &mut Context<Self>) -> Div {
        let mut panel = div().flex().flex_col().gap_2();
        if message.deleted {
            return panel;
        }
        let conversation = message.conversation_id;
        let mid = message.id;
        for attachment in &message.attachments {
            let id = attachment.id.clone();
            let filename = attachment.filename.clone();
            let mut card = div()
                .p_3()
                .rounded_lg()
                .bg(rgb(RAISED))
                .flex()
                .flex_col()
                .gap_2()
                .child(heading(filename.clone()).whitespace_normal())
                .child(small(format!(
                    "{} bytes · {}",
                    attachment.bytes, attachment.mime
                )));
            let save_id = id.clone();
            let mut controls = div().flex().gap_2().flex_wrap().child(
                Button::new(SharedString::from(format!("save-{mid}-{id}")))
                    .small()
                    .label("Save file")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_file(
                            conversation,
                            mid,
                            save_id.clone(),
                            filename.clone(),
                            window,
                            cx,
                        );
                    })),
            );
            if matches!(
                attachment.mime.as_str(),
                "image/png" | "image/jpeg" | "image/webp" | "video/mp4" | "video/webm"
            ) {
                let preview_id = id.clone();
                controls = controls.child(
                    Button::new(SharedString::from(format!("preview-{mid}-{id}")))
                        .small()
                        .ghost()
                        .label("Load preview")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(
                                Command::Preview {
                                    conversation,
                                    message: mid,
                                    file: preview_id.clone(),
                                },
                                cx,
                            );
                        })),
                );
            }
            card = card.child(controls);
            if let Some(bytes) = self.snapshot.previews.get(&id) {
                card = card.child(
                    img(ImageSource::Image(Arc::new(Image::from_bytes(
                        ImageFormat::Png,
                        (**bytes).clone(),
                    ))))
                    .max_w(px(480.))
                    .max_h(px(320.))
                    .object_fit(ObjectFit::Contain),
                );
            }
            panel = panel.child(card);
        }
        if let Some(MessageContent::Markdown { text }) = &message.content {
            let mut seen = std::collections::HashSet::new();
            for event in pulldown_cmark::Parser::new(text) {
                if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                    dest_url, ..
                }) = event
                {
                    let url = dest_url.to_string();
                    if !url.starts_with("https://") || url.len() > 2048 || !seen.insert(url.clone())
                    {
                        continue;
                    }
                    let slot = seen.len();
                    panel = panel.child(
                        Button::new(SharedString::from(format!("embed-{mid}-{slot}")))
                            .small()
                            .ghost()
                            .label("Load link details (contacts website)")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.command(
                                    Command::Embed {
                                        conversation,
                                        message: mid,
                                        url: url.clone(),
                                    },
                                    cx,
                                );
                            })),
                    );
                    if slot >= 3 {
                        break;
                    }
                }
            }
        }
        if let Some(embed) = self.snapshot.embeds.get(&mid) {
            panel = panel.child(
                div()
                    .p_3()
                    .border_l_2()
                    .border_color(rgb(AMBER))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(heading(embed.title.clone()).whitespace_normal())
                    .child(small(embed.description.clone()).whitespace_normal())
                    .child(small(embed.url.clone()).whitespace_normal()),
            );
        }
        panel
    }
    pub(super) fn transfers(&self, cx: &mut Context<Self>) -> Div {
        let mut panel = div().flex().flex_col().gap_2();
        for upload in self
            .snapshot
            .uploads
            .iter()
            .filter(|u| Some(u.status.conversation_id) == self.snapshot.selected)
        {
            let status = &upload.status;
            // A durable message in the outbox already owns this file; never
            // offer to send or delete it a second time while awaiting ack.
            let queued = self
                .snapshot
                .pending
                .iter()
                .any(|p| p.request.attachments.contains(&status.id));
            let attached = self
                .snapshot
                .messages
                .iter()
                .any(|m| m.attachments.iter().any(|a| a.id == status.id));
            if queued || attached || status.state == UploadState::Attached {
                continue;
            }
            let id = status.id.clone();
            let cancel = id.clone();
            let conversation = status.conversation_id;
            let active = self.snapshot.transferring.contains(&id);
            let mut controls = div().flex().flex_wrap().gap_2();
            if status.state == UploadState::Ready {
                controls = controls.child(
                    Button::new(SharedString::from(format!("send-file-{id}")))
                        .small()
                        .primary()
                        .label("Send file")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(
                                Command::SendFile {
                                    conversation,
                                    upload: id.clone(),
                                },
                                cx,
                            );
                        })),
                );
            } else {
                controls = controls.child(
                    Button::new(SharedString::from(format!("resume-file-{id}")))
                        .small()
                        .label(if active {
                            "Uploading…"
                        } else if status.received == 0 {
                            "Upload"
                        } else {
                            "Resume"
                        })
                        .disabled(active)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.command(Command::ResumeUpload(id.clone()), cx);
                        })),
                );
            }
            controls = controls.child(
                Button::new(SharedString::from(format!("cancel-file-{cancel}")))
                    .small()
                    .ghost()
                    .label("Cancel file")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.command(Command::CancelUpload(cancel.clone()), cx);
                    })),
            );
            let progress = status.received as f32 / status.bytes as f32;
            panel = panel.child(
                div()
                    .bg(rgb(RAISED))
                    .rounded_lg()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        small(format!(
                            "{} · {} / {} bytes · {:?}",
                            status.filename, status.received, status.bytes, status.state
                        ))
                        .whitespace_normal(),
                    )
                    .child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .bg(rgb(BORDER))
                            .child(div().h_full().w(relative(progress)).bg(rgb(AMBER))),
                    )
                    .child(controls),
            );
        }
        if let Some(saved) = &self.snapshot.saved {
            panel = panel.child(small(saved.clone()));
        }
        panel
    }
}
