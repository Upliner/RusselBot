//! The presence of the bare domain and of `local@`, worked out from the
//! bot's state, and what has to be sent when it changes.

use super::stanza::Shown;

/// What the owners see of the two addresses that have a presence.
#[derive(Debug, Clone, PartialEq)]
pub struct Presence {
    pub domain: Shown,
    pub local: Shown,
}

pub const NOT_IN_A_REGION: &str = "not in a region";

/// The presence for the bot's state: the region the avatar is in, if any,
/// and whether the bot is stopping.
pub fn presence(region: Option<&str>, stopping: bool) -> Presence {
    match (stopping, region) {
        (true, _) => Presence {
            domain: Shown::Unavailable,
            local: Shown::Unavailable,
        },
        (false, Some(region)) => Presence {
            domain: Shown::Available(Some(region.to_string())),
            local: Shown::Available(None),
        },
        (false, None) => Presence {
            domain: Shown::Away(NOT_IN_A_REGION.to_string()),
            local: Shown::Unavailable,
        },
    }
}

/// What to send to go from `last` (nothing sent yet: `None`) to `now`: the
/// local part (`None` for the bare domain) and its presence, for each
/// address that changed.
pub fn changes(last: Option<&Presence>, now: &Presence) -> Vec<(Option<&'static str>, Shown)> {
    let mut out = Vec::new();
    if last.is_none_or(|last| last.domain != now.domain) {
        out.push((None, now.domain.clone()));
    }
    if last.is_none_or(|last| last.local != now.local) {
        out.push((Some(super::address::LOCAL), now.local.clone()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_rows() {
        assert_eq!(
            presence(Some("Home"), false),
            Presence {
                domain: Shown::Available(Some("Home".into())),
                local: Shown::Available(None)
            }
        );
        assert_eq!(
            presence(None, false),
            Presence {
                domain: Shown::Away("not in a region".into()),
                local: Shown::Unavailable
            }
        );
        for region in [Some("Home"), None] {
            assert_eq!(
                presence(region, true),
                Presence {
                    domain: Shown::Unavailable,
                    local: Shown::Unavailable
                }
            );
        }
    }

    #[test]
    fn everything_is_sent_first_and_then_only_what_changed() {
        let home = presence(Some("Home"), false);
        assert_eq!(changes(None, &home).len(), 2);
        assert_eq!(changes(Some(&home), &home), []);
        let away = presence(Some("Away"), false);
        assert_eq!(changes(Some(&home), &away), [(None, Shown::Available(Some("Away".into())))]);
        let lost = presence(None, false);
        assert_eq!(
            changes(Some(&away), &lost),
            [(None, Shown::Away("not in a region".into())), (Some("local"), Shown::Unavailable)]
        );
    }
}
