use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "porch/paperclip.svg" => Some(include_bytes!("icons/paperclip.svg")),
            "porch/search.svg" => Some(include_bytes!("icons/search.svg")),
            "porch/hash.svg" => Some(include_bytes!("icons/hash.svg")),
            "porch/volume.svg" => Some(include_bytes!("icons/volume.svg")),
            "porch/settings.svg" => Some(include_bytes!("icons/settings.svg")),
            "porch/inbox.svg" => Some(include_bytes!("icons/inbox.svg")),
            "porch/user.svg" => Some(include_bytes!("icons/user.svg")),
            "porch/plus.svg" => Some(include_bytes!("icons/plus.svg")),
            "porch/send.svg" => Some(include_bytes!("icons/send.svg")),
            "porch/reply.svg" => Some(include_bytes!("icons/reply.svg")),
            "porch/download.svg" => Some(include_bytes!("icons/download.svg")),
            "porch/edit.svg" => Some(include_bytes!("icons/edit.svg")),
            "porch/heart.svg" => Some(include_bytes!("icons/heart.svg")),
            "porch/chevron-down.svg" => Some(include_bytes!("icons/chevron-down.svg")),
            "porch/chevron-right.svg" => Some(include_bytes!("icons/chevron-right.svg")),
            "porch/mic.svg" => Some(include_bytes!("icons/mic.svg")),
            "porch/mic-off.svg" => Some(include_bytes!("icons/mic-off.svg")),
            "porch/headphones.svg" => Some(include_bytes!("icons/headphones.svg")),
            "porch/leave.svg" => Some(include_bytes!("icons/leave.svg")),
            "porch/pin.svg" => Some(include_bytes!("icons/pin.svg")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}
