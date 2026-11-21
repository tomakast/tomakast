//! The action panel by mouse: a click on one of its rows picks that entry.

use gpui_kit::{ClickEvent, Context, Window};

use super::LauncherView;

impl LauncherView {
    /// A click on row `ix` of the open panel (an index among the rows it
    /// shows): the row is selected and its entry runs, as Enter would run it.
    pub(crate) fn panel_row_clicked(
        &mut self,
        ix: usize,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &mut self.panel else {
            return;
        };
        if !panel.select(ix) {
            return;
        }
        cx.notify();
        self.confirm_panel(window, cx);
    }
}
