//! UI language: English and Traditional Chinese, switched at runtime with `L`.
//!
//! Must NOT: translate program identifiers (task ids, branch names, commands).

/// Languages the TUI can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    ZhTw,
}

impl Language {
    /// The language after pressing `L`.
    pub fn toggled(self) -> Language {
        match self {
            Language::En => Language::ZhTw,
            Language::ZhTw => Language::En,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_cycles_between_the_two_languages() {
        assert_eq!(Language::En.toggled(), Language::ZhTw);
        assert_eq!(Language::En.toggled().toggled(), Language::En);
    }
}
