//! Scoped CSS for the i18n-demo app.
//!
//! Class names are unhashed constants — `stylance-cli` is not part of the
//! build pipeline. Selectors are paired via `[data-i18n-demo]` attribute
//! on the root element of `app.rs`.

/// Class for the locale switch buttons.
pub const LOCALE_BTN: &str = "locale-btn";

/// Class for the search input field.
pub const SEARCH_INPUT: &str = "search-input";

/// Class for the increment/decrement counter buttons.
pub const COUNTER_BTN: &str = "counter-btn";

/// Class for the click-count display span.
pub const CLICK_COUNT: &str = "click-count";

/// Returns the CSS for the i18n-demo, wrapped in a `<style>` tag at runtime.
#[must_use]
pub const fn home_css() -> &'static str {
    include_str!("styles/home.module.css")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_btn_constant_is_unhashed() {
        assert_eq!(LOCALE_BTN, "locale-btn");
    }

    #[test]
    fn search_input_constant_is_unhashed() {
        assert_eq!(SEARCH_INPUT, "search-input");
    }

    #[test]
    fn counter_btn_constant_is_unhashed() {
        assert_eq!(COUNTER_BTN, "counter-btn");
    }

    #[test]
    fn click_count_constant_is_unhashed() {
        assert_eq!(CLICK_COUNT, "click-count");
    }

    #[test]
    fn home_module_css_has_no_global_pseudo_class() {
        let css = include_str!("styles/home.module.css");
        assert!(
            !css.contains(":global("),
            "CSS must not use :global() — not a real CSS pseudo-class"
        );
    }

    #[test]
    fn home_module_css_uses_data_attribute_selector() {
        let css = include_str!("styles/home.module.css");
        assert!(
            css.contains("[data-i18n-demo]"),
            "CSS must use [data-i18n-demo] attribute selector"
        );
        assert!(
            !css.contains("#i18n-demo"),
            "CSS must not use #i18n-demo — no element has that id"
        );
    }

    #[test]
    fn constants_match_css_selectors() {
        let css = include_str!("styles/home.module.css");
        assert!(css.contains(".locale-btn"));
        assert!(css.contains(".search-input"));
        assert!(css.contains(".counter-btn"));
        assert!(css.contains(".click-count"));
    }
}
