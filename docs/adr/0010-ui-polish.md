# Evening Threshold native polish

Warm pine surfaces, compact Manrope hierarchy, cream conversation text and restrained amber selection. The familiar four-column layout stays; avatars and geometry make navigation distinguishable without pretending to have profile or presence APIs.

Presentation only. Message groups require matching author, valid timestamps less than five minutes apart, and no reply/deleted boundary. Local short clocks replace raw ISO strings. Actions occupy a floating hover toolbar instead of permanently stealing message width. Category collapse is local view state. Member groups use actual strongest roles; no invented online status or remote profile names. Voice roster appears only for the connected channel because that is the snapshot available today.

Original embedded line icons avoid runtime network/font dependencies. Dark first; existing brand light tokens unchanged. Headless screenshots demonstrate native rendering, not Windows runtime, DPI, screen-reader or IME acceptance.

Follow-up uses Sharkord's layout hierarchy, studied read-only in porch-legacy: 288px channel sidebar, 240px member list, 48px shared headers, 56px user footer, compact message groups and integrated attachment/send composer. Native Rust/GPUI implementation, no React source copied; MIT source notice retained in NOTICE. All decorative/explanatory panel copy removed. Input placeholder follows selected channel. Manrope medium body / semibold names. Secondary #CAC7B9 has 8.58:1 minimum contrast across the four dark surfaces (#111210/#191D19/#232B24/#20271F), computed using WCAG relative luminance. Compact non-preview file cards are capped at344px with an icon-only save action. Media retains504px cap.

The previous thin white outer line belongs to Openbox's server-side frame, not Porch's client; conversation/hover evidence now captures the native client window directly, without that OS frame. Client-decorated GPUI border also explicitly uses the Porch border token rather than inherited theme defaults. Windows/native frame appearance remains platform-controlled.
