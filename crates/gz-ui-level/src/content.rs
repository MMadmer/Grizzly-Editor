//! The two panels whose bodies other crates draw: the Content Browser (`gz-ui-content`) and the
//! Output Log (`gz-ui-app`). The Level dock hosts them in the Wide layout (or wherever the
//! author docked them); the same objects show in the drawers otherwise.

use egui::Ui;
use gz_app::DocCx;

/// The Content Browser docked on Level.
pub fn content_browser(ui: &mut Ui, cx: &mut DocCx<'_>) {
    gz_ui_content::content_browser(ui, cx, gz_ui_content::Host::Panel);
}

/// The Output Log docked on Level.
pub fn output_log(ui: &mut Ui, cx: &mut DocCx<'_>) {
    gz_ui_app::output_log(ui, cx.editor, gz_ui_app::LogHost::Panel);
}
