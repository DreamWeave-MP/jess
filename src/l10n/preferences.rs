//! Where a locale preference comes from, and the order the sources are honored in.
//!
//! The registry takes one ordered list ([`Registry::set_preferred_locales`]). This module is the
//! policy that builds it: what the application was told outranks what the user configured, which
//! outranks what the system reports. A bundle's fallback locale is not a preference; the
//! registry tries it after the whole list.
//!
//! [`Registry::set_preferred_locales`]: crate::l10n::Registry::set_preferred_locales

use crate::l10n::locale::Locale;

/// The sources of a locale preference, highest precedence first.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Preferences {
    /// What the application was told outright: a command-line flag, a call from privileged code.
    pub explicit: Vec<Locale>,
    /// What the user configured: a profile or settings file.
    pub user: Vec<Locale>,
    /// What the platform reports; [`system`] reads the POSIX environment.
    pub system: Vec<Locale>,
}

impl Preferences {
    /// The locales in precedence order, each once: explicit, then user, then system.
    #[must_use]
    pub fn resolve(&self) -> Vec<Locale> {
        let mut resolved: Vec<Locale> = Vec::new();
        for locale in self.explicit.iter().chain(&self.user).chain(&self.system) {
            if !resolved.contains(locale) {
                resolved.push(locale.clone());
            }
        }
        resolved
    }
}

/// The locale a POSIX locale name means: `pt_BR.UTF-8@latin` is `pt-BR`. `C`, `POSIX`, and
/// anything that isn't a locale are `None`.
#[must_use]
pub fn from_posix(name: &str) -> Option<Locale> {
    let name = name.split(['.', '@']).next()?;
    if name.is_empty() || name == "C" || name == "POSIX" {
        return None;
    }
    Locale::parse(name).ok()
}

/// The locales the POSIX environment asks for, in gettext's order: every entry of `LANGUAGE`,
/// then the first of `LC_ALL`, `LC_MESSAGES`, and `LANG` that is set.
///
/// This reads environment variables and nothing else. A platform that reports its locale another
/// way (Windows, macOS) is the host's to ask; it passes the answer as [`Preferences::system`].
#[must_use]
pub fn system() -> Vec<Locale> {
    system_from(|name| std::env::var(name).ok())
}

/// [`system`] over any lookup, so the rule can be tested without touching the environment.
pub fn system_from(lookup: impl Fn(&str) -> Option<String>) -> Vec<Locale> {
    let mut locales: Vec<Locale> = Vec::new();
    let mut push = |locale: Option<Locale>| {
        if let Some(locale) = locale
            && !locales.contains(&locale)
        {
            locales.push(locale);
        }
    };
    if let Some(languages) = lookup("LANGUAGE") {
        for name in languages.split(':') {
            push(from_posix(name));
        }
    }
    let first_set = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| lookup(name).filter(|value| !value.is_empty()));
    if let Some(name) = first_set {
        push(from_posix(&name));
    }
    locales
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(locales: &[Locale]) -> Vec<&str> {
        locales.iter().map(Locale::as_str).collect()
    }

    #[test]
    fn posix_names() {
        assert_eq!(from_posix("pt_BR.UTF-8").unwrap().as_str(), "pt-BR");
        assert_eq!(from_posix("sr_RS@latin").unwrap().as_str(), "sr-RS");
        assert_eq!(from_posix("de").unwrap().as_str(), "de");
        assert_eq!(from_posix("C"), None);
        assert_eq!(from_posix("POSIX"), None);
        assert_eq!(from_posix("C.UTF-8"), None);
        assert_eq!(from_posix(""), None);
    }

    #[test]
    fn environment_order() {
        let environment = |name: &str| match name {
            "LANGUAGE" => Some("de_AT:de:en".to_owned()),
            "LC_MESSAGES" => Some("fr_FR.UTF-8".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        };
        assert_eq!(tags(&system_from(environment)), ["de-AT", "de", "en", "fr-FR"]);
        assert_eq!(
            tags(&system_from(|name| (name == "LANG").then(|| "pl_PL.UTF-8".to_owned()))),
            ["pl-PL"]
        );
        assert_eq!(system_from(|_| None), []);
        assert_eq!(system_from(|name| (name == "LANG").then(|| "C".to_owned())), []);
    }

    #[test]
    fn explicit_beats_user_beats_system() {
        let locale = |tag: &str| Locale::parse(tag).unwrap();
        let preferences = Preferences {
            explicit: vec![locale("pl")],
            user: vec![locale("de"), locale("pl")],
            system: vec![locale("en-US"), locale("de")],
        };
        assert_eq!(tags(&preferences.resolve()), ["pl", "de", "en-US"]);
        assert_eq!(Preferences::default().resolve(), []);
    }
}
