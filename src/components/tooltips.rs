//! Tooltips which show at once when clicked, for every element with a title, see assets/scripts/tooltips.js, in each
//! window which has one of these, as main.css shows which have one.

use dioxus::prelude::*;

const SCRIPT: &str = include_str!("../../assets/scripts/tooltips.js");

#[component]
pub fn Tooltips() -> Element {
    use_hook(|| {
        let _ = document::eval(SCRIPT);
    });
    rsx! {}
}
