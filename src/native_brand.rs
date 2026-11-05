use std::sync::{Arc, OnceLock};

use eframe::egui;

const MARK_PNG: &[u8] = include_bytes!("../assets/branding/bookmark-bsync-cat-card-3d.png");
const MARK_NAME: &str = "Bookmark Bsync cat bookmark logo";

pub(crate) fn window_icon() -> Result<Arc<egui::IconData>, String> {
    static ICON: OnceLock<Result<Arc<egui::IconData>, String>> = OnceLock::new();
    ICON.get_or_init(|| {
        eframe::icon_data::from_png_bytes(MARK_PNG)
            .map(Arc::new)
            .map_err(|error| format!("Could not decode the embedded Bookmark Bsync logo: {error}"))
    })
    .clone()
}

fn texture(ctx: &egui::Context) -> Result<egui::TextureHandle, String> {
    let id = egui::Id::unique("bookmark-bsync-brand-logo");
    if let Some(cached) = ctx.data(|data| data.get_temp::<Result<egui::TextureHandle, String>>(id))
    {
        return cached;
    }
    let result = window_icon().map(|icon| {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        ctx.load_texture(MARK_NAME, image, egui::TextureOptions::LINEAR)
    });
    ctx.data_mut(|data| data.insert_temp(id, result.clone()));
    result
}

pub(crate) fn app_mark(ui: &mut egui::Ui, max_side: f32) -> egui::Response {
    match texture(ui.ctx()) {
        Ok(texture) => ui
            .add(
                egui::Image::new(&texture)
                    .fit_to_exact_size(egui::Vec2::splat(max_side))
                    .maintain_aspect_ratio(true)
                    .alt_text(MARK_NAME),
            )
            .on_hover_text(MARK_NAME),
        Err(error) => ui.label("Bookmark Bsync").on_hover_text(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_mark_decodes_and_reuses_icon() -> Result<(), String> {
        let first = window_icon()?;
        let second = window_icon()?;
        assert!(first.width > 0 && first.height > 0);
        assert_eq!(
            first.rgba.len(),
            first.width as usize * first.height as usize * 4
        );
        assert!(Arc::ptr_eq(&first, &second));
        Ok(())
    }

    #[test]
    fn texture_is_reused_within_a_context() -> Result<(), String> {
        let ctx = egui::Context::default();
        let first = texture(&ctx)?;
        let second = texture(&ctx)?;
        assert_eq!(first.id(), second.id());
        Ok(())
    }

    #[test]
    fn app_mark_stays_within_requested_size() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = app_mark(ui, 36.0);
            assert!(response.rect.width() > 0.0 && response.rect.width() <= 36.0);
            assert!(response.rect.height() > 0.0 && response.rect.height() <= 36.0);
        });
        output.textures_delta.clear();
    }
}
