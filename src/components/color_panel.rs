//! On macOS the "Show Colors…" window of a colour input is the system colour panel, which is shared by the whole app
//! and opens wherever it was last left, often on a different display. Moving it to the mouse before it opens keeps it
//! next to the colour picker.

/// Move the system colour panel to the mouse, unless it is already open. Does nothing on other platforms.
pub fn move_color_panel_to_mouse() {
    #[cfg(target_os = "macos")]
    macos::move_color_panel_to_mouse();
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSColorPanel, NSEvent, NSScreen};
    use objc2_foundation::{NSPoint, NSRect};

    /// How far right of the mouse the panel opens, so that it does not cover the swatches it was opened from.
    const OFFSET_FROM_MOUSE: f64 = 24.0;

    fn contains(rect: NSRect, point: NSPoint) -> bool {
        point.x >= rect.origin.x
            && point.x < rect.origin.x + rect.size.width
            && point.y >= rect.origin.y
            && point.y < rect.origin.y + rect.size.height
    }

    pub fn move_color_panel_to_mouse() {
        // AppKit must only be used on the main thread, which is where Dioxus desktop runs event handlers.
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let panel = NSColorPanel::sharedColorPanel(mtm);
        if panel.isVisible() {
            // The user may have moved it, so leave it where it is.
            return;
        }

        // Screen coordinates have their origin at the bottom left, so the top of the panel is at the mouse.
        let mouse = NSEvent::mouseLocation();
        let size = panel.frame().size;
        let mut top_left = NSPoint::new(mouse.x + OFFSET_FROM_MOUSE, mouse.y);

        // Keep it on the screen the mouse is on.
        if let Some(screen) = NSScreen::screens(mtm)
            .iter()
            .find(|screen| contains(screen.frame(), mouse))
        {
            let visible = screen.visibleFrame();
            top_left.x = top_left
                .x
                .min(visible.origin.x + visible.size.width - size.width)
                .max(visible.origin.x);
            top_left.y = top_left
                .y
                .max(visible.origin.y + size.height)
                .min(visible.origin.y + visible.size.height);
        }

        panel.setFrameTopLeftPoint(top_left);
    }
}
