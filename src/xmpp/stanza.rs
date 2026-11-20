//! The stanzas the gateway writes, built as text, and the ones it reads,
//! sorted out of the stream's elements.

use super::stream::{Element, NS_COMPONENT};
use crate::log::format_timestamp;

pub const NS_STANZAS: &str = "urn:ietf:params:xml:ns:xmpp-stanzas";
pub const NS_DISCO_INFO: &str = "http://jabber.org/protocol/disco#info";
pub const NS_PING: &str = "urn:xmpp:ping";
pub const NS_NICK: &str = "http://jabber.org/protocol/nick";
pub const NS_DELAY: &str = "urn:xmpp:delay";

/// Whether XML 1.0 allows `ch` in a document at all.
fn allowed(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// Text as element content: markup characters escaped, and characters XML
/// cannot carry left out. `>` is escaped too, which keeps `]]>` harmless.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars().filter(|ch| allowed(*ch)) {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// A value as a quoted attribute.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars().filter(|ch| allowed(*ch)) {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            // Kept as they are, not folded into spaces.
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

/// A one-to-one chat message. `delay` is when it was sent or recorded, in
/// seconds since the Unix epoch, for a message that did not go out at once.
pub fn chat_message(from: &str, to: &str, body: &str, nick: Option<&str>, delay: Option<u64>) -> String {
    let mut out = format!(
        "<message type='chat' from={} to={}><body>{}</body>",
        quote(from),
        quote(to),
        escape_text(body)
    );
    if let Some(nick) = nick.filter(|nick| !nick.is_empty()) {
        out.push_str(&format!("<nick xmlns='{NS_NICK}'>{}</nick>", escape_text(nick)));
    }
    if let Some(time) = delay {
        let domain = from.rsplit('@').next().unwrap_or(from);
        out.push_str(&format!(
            "<delay xmlns='{NS_DELAY}' from={} stamp='{}'/>",
            quote(domain),
            format_timestamp(time)
        ));
    }
    out.push_str("</message>");
    out
}

/// The stanza error conditions the gateway answers with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Condition {
    Forbidden,
    ItemNotFound,
    FeatureNotImplemented,
    NotAcceptable,
    ServiceUnavailable,
}

impl Condition {
    pub fn name(self) -> &'static str {
        match self {
            Condition::Forbidden => "forbidden",
            Condition::ItemNotFound => "item-not-found",
            Condition::FeatureNotImplemented => "feature-not-implemented",
            Condition::NotAcceptable => "not-acceptable",
            Condition::ServiceUnavailable => "service-unavailable",
        }
    }

    fn error_type(self) -> &'static str {
        match self {
            Condition::Forbidden => "auth",
            Condition::NotAcceptable => "modify",
            _ => "cancel",
        }
    }
}

/// The error answer to a stanza of kind `stanza` (`message`, `iq`, ...)
/// that `to` sent to `from` with `id`.
pub fn error_reply(stanza: &str, from: &str, to: &str, id: Option<&str>, condition: Condition, text: &str) -> String {
    let id = id.map(|id| format!(" id={}", quote(id))).unwrap_or_default();
    let text = match text.is_empty() {
        true => String::new(),
        false => format!("<text xmlns='{NS_STANZAS}' xml:lang='en'>{}</text>", escape_text(text)),
    };
    format!(
        "<{stanza} type='error' from={} to={}{id}><error type='{}'><{} xmlns='{NS_STANZAS}'/>{text}</error></{stanza}>",
        quote(from),
        quote(to),
        condition.error_type(),
        condition.name()
    )
}

/// What an address shows as its presence.
#[derive(Debug, Clone, PartialEq)]
pub enum Shown {
    Available(Option<String>),
    Away(String),
    Unavailable,
}

pub fn presence(from: &str, to: &str, shown: &Shown) -> String {
    let (from, to) = (quote(from), quote(to));
    let status = |text: &str| format!("<status>{}</status>", escape_text(text));
    match shown {
        Shown::Available(None) => format!("<presence from={from} to={to}/>"),
        Shown::Available(Some(text)) => format!("<presence from={from} to={to}>{}</presence>", status(text)),
        Shown::Away(text) => format!("<presence from={from} to={to}><show>away</show>{}</presence>", status(text)),
        Shown::Unavailable => format!("<presence type='unavailable' from={from} to={to}/>"),
    }
}

/// A `subscribed` or `unsubscribed` answer.
pub fn subscription(from: &str, to: &str, approved: bool) -> String {
    let kind = if approved { "subscribed" } else { "unsubscribed" };
    format!("<presence type='{kind}' from={} to={}/>", quote(from), quote(to))
}

pub fn disco_info_result(from: &str, to: &str, id: &str) -> String {
    format!(
        "<iq type='result' from={} to={} id={}><query xmlns='{NS_DISCO_INFO}'>\
         <identity category='gateway' type='secondlife' name='RusselBot'/>\
         <feature var='{NS_DISCO_INFO}'/><feature var='{NS_PING}'/><feature var='{NS_NICK}'/>\
         <feature var='{NS_DELAY}'/></query></iq>",
        quote(from),
        quote(to),
        quote(id)
    )
}

/// An empty `result`, which is the answer to a ping.
pub fn iq_result(from: &str, to: &str, id: &str) -> String {
    format!("<iq type='result' from={} to={} id={}/>", quote(from), quote(to), quote(id))
}

// ------------------------------------------------------------------ input

/// A stanza from the server, as far as the gateway cares.
#[derive(Debug, Clone, PartialEq)]
pub enum Stanza {
    Message {
        kind: String,
        from: Option<String>,
        to: String,
        id: Option<String>,
        /// `None` for a message without a body, such as a chat state.
        body: Option<String>,
    },
    Presence {
        /// `None` for available presence.
        kind: Option<String>,
        from: Option<String>,
        to: String,
    },
    Iq {
        kind: String,
        id: Option<String>,
        from: Option<String>,
        to: String,
        /// The namespace and name of the request's child.
        child: Option<(String, String)>,
    },
    /// A stanza of type `error`, which is never answered.
    Error,
    /// Anything else on the stream.
    Other(String),
}

pub fn classify(element: &Element) -> Stanza {
    let attr = |name: &str| element.attr(name).map(str::to_string);
    if element.ns != NS_COMPONENT {
        return Stanza::Other(element.name.clone());
    }
    if element.attr("type") == Some("error") {
        return Stanza::Error;
    }
    let to = attr("to").unwrap_or_default();
    match element.name.as_str() {
        "message" => Stanza::Message {
            kind: attr("type").unwrap_or_else(|| "normal".to_string()),
            from: attr("from"),
            to,
            id: attr("id"),
            body: element.child("body", NS_COMPONENT).map(|body| body.text.clone()),
        },
        "presence" => Stanza::Presence {
            kind: attr("type"),
            from: attr("from"),
            to,
        },
        "iq" => Stanza::Iq {
            kind: attr("type").unwrap_or_default(),
            id: attr("id"),
            from: attr("from"),
            to,
            child: element.children.first().map(|child| (child.ns.clone(), child.name.clone())),
        },
        other => Stanza::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xmpp::stream::StreamReader;
    use std::io::Cursor;

    const T: u64 = 1_795_021_463; // 2026-11-18T17:04:23Z

    #[test]
    fn text_and_attributes_are_escaped() {
        assert_eq!(escape_text("a < b & c ]]> d"), "a &lt; b &amp; c ]]&gt; d");
        assert_eq!(escape_text("bell\u{7}\u{FFFE} tab\tok"), "bell tab\tok");
        assert_eq!(quote("it's \"x\" <&>\n"), "'it&apos;s &quot;x&quot; &lt;&amp;&gt;&#10;'");
    }

    #[test]
    fn chat_messages() {
        assert_eq!(
            chat_message("jane.doe@bot.rig.test", "owner@rig.test", "1 < 2 & ]]>", Some("Janey"), None),
            "<message type='chat' from='jane.doe@bot.rig.test' to='owner@rig.test'><body>1 &lt; 2 &amp; ]]&gt;</body>\
             <nick xmlns='http://jabber.org/protocol/nick'>Janey</nick></message>"
        );
        assert_eq!(
            chat_message("local@bot.rig.test", "owner@rig.test", "hi", None, Some(T)),
            "<message type='chat' from='local@bot.rig.test' to='owner@rig.test'><body>hi</body>\
             <delay xmlns='urn:xmpp:delay' from='bot.rig.test' stamp='2026-11-18T17:04:23Z'/></message>"
        );
        assert_eq!(
            chat_message("bot.rig.test", "owner@rig.test", "ok", Some(""), Some(T)),
            "<message type='chat' from='bot.rig.test' to='owner@rig.test'><body>ok</body>\
             <delay xmlns='urn:xmpp:delay' from='bot.rig.test' stamp='2026-11-18T17:04:23Z'/></message>"
        );
    }

    #[test]
    fn errors() {
        assert_eq!(
            error_reply(
                "message",
                "jane.doe@bot.rig.test",
                "x@y/z",
                Some("m1"),
                Condition::Forbidden,
                "only <owners>"
            ),
            "<message type='error' from='jane.doe@bot.rig.test' to='x@y/z' id='m1'><error type='auth'>\
             <forbidden xmlns='urn:ietf:params:xml:ns:xmpp-stanzas'/>\
             <text xmlns='urn:ietf:params:xml:ns:xmpp-stanzas' xml:lang='en'>only &lt;owners&gt;</text></error></message>"
        );
        assert_eq!(
            error_reply("iq", "bot.rig.test", "x@y/z", None, Condition::ServiceUnavailable, ""),
            "<iq type='error' from='bot.rig.test' to='x@y/z'><error type='cancel'>\
             <service-unavailable xmlns='urn:ietf:params:xml:ns:xmpp-stanzas'/></error></iq>"
        );
        assert!(error_reply("message", "a", "b", None, Condition::NotAcceptable, "").contains("type='modify'"));
        assert!(error_reply("message", "a", "b", None, Condition::ItemNotFound, "").contains("<item-not-found "));
        assert!(error_reply("message", "a", "b", None, Condition::FeatureNotImplemented, "").contains("type='cancel'"));
    }

    #[test]
    fn presences_and_subscriptions() {
        assert_eq!(
            presence("bot.rig.test", "o@rig.test", &Shown::Available(Some("Home & Away".into()))),
            "<presence from='bot.rig.test' to='o@rig.test'><status>Home &amp; Away</status></presence>"
        );
        assert_eq!(
            presence("local@bot.rig.test", "o@rig.test", &Shown::Available(None)),
            "<presence from='local@bot.rig.test' to='o@rig.test'/>"
        );
        assert_eq!(
            presence("bot.rig.test", "o@rig.test", &Shown::Away("not in a region".into())),
            "<presence from='bot.rig.test' to='o@rig.test'><show>away</show><status>not in a region</status></presence>"
        );
        assert_eq!(
            presence("bot.rig.test", "o@rig.test", &Shown::Unavailable),
            "<presence type='unavailable' from='bot.rig.test' to='o@rig.test'/>"
        );
        assert_eq!(
            subscription("bot.rig.test", "o@rig.test", true),
            "<presence type='subscribed' from='bot.rig.test' to='o@rig.test'/>"
        );
        assert_eq!(
            subscription("bot.rig.test", "s@rig.test", false),
            "<presence type='unsubscribed' from='bot.rig.test' to='s@rig.test'/>"
        );
    }

    #[test]
    fn iq_answers() {
        let disco = disco_info_result("bot.rig.test", "o@rig.test/r", "q1");
        assert!(disco.starts_with("<iq type='result' from='bot.rig.test' to='o@rig.test/r' id='q1'>"));
        assert!(disco.contains("<identity category='gateway'"));
        assert!(disco.contains("<feature var='urn:xmpp:ping'/>"));
        assert_eq!(
            iq_result("bot.rig.test", "o@rig.test/r", "p1"),
            "<iq type='result' from='bot.rig.test' to='o@rig.test/r' id='p1'/>"
        );
    }

    /// Reads stanzas as the rig's Prosody sends them to the component.
    fn stanzas(text: &str) -> Vec<Stanza> {
        let header = "<?xml version='1.0'?><stream:stream xmlns:stream='http://etherx.jabber.org/streams' \
            xml:lang='en' xmlns='jabber:component:accept' id='x' from='bot.rig.test'>";
        let mut reader = StreamReader::new(Cursor::new(format!("{header}{text}").into_bytes()));
        reader.read_header().unwrap();
        std::iter::from_fn(|| reader.next_stanza().ok())
            .map(|element| classify(&element))
            .collect()
    }

    #[test]
    fn stanzas_captured_on_the_rig_are_sorted_out() {
        let read = stanzas(
            "<message type='chat' from='owner@rig.test/rig' xml:lang='en' id='m1' to='bot.rig.test'><body>status</body></message>\
             <message type='chat' from='owner@rig.test/gajim' to='jane.doe@bot.rig.test' id='c2'>\
             <composing xmlns='http://jabber.org/protocol/chatstates'/></message>\
             <presence type='subscribe' from='owner@rig.test' to='bot.rig.test'/>\
             <presence from='owner@rig.test/rig' xml:lang='en' to='bot.rig.test'/>\
             <presence type='probe' from='owner@rig.test' to='local@bot.rig.test'/>\
             <iq type='get' from='owner@rig.test/rig' to='bot.rig.test' id='q1'><query xmlns='http://jabber.org/protocol/disco#info'/></iq>\
             <iq type='get' from='owner@rig.test/rig' to='jane.doe@bot.rig.test' id='q2'><vCard xmlns='vcard-temp'/></iq>\
             <message type='error' from='owner@rig.test' to='bot.rig.test'><error type='cancel'/></message>\
             <iq type='error' from='owner@rig.test' to='bot.rig.test' id='x'/>",
        );
        assert_eq!(
            read,
            vec![
                Stanza::Message {
                    kind: "chat".into(),
                    from: Some("owner@rig.test/rig".into()),
                    to: "bot.rig.test".into(),
                    id: Some("m1".into()),
                    body: Some("status".into()),
                },
                Stanza::Message {
                    kind: "chat".into(),
                    from: Some("owner@rig.test/gajim".into()),
                    to: "jane.doe@bot.rig.test".into(),
                    id: Some("c2".into()),
                    body: None,
                },
                Stanza::Presence {
                    kind: Some("subscribe".into()),
                    from: Some("owner@rig.test".into()),
                    to: "bot.rig.test".into(),
                },
                Stanza::Presence {
                    kind: None,
                    from: Some("owner@rig.test/rig".into()),
                    to: "bot.rig.test".into(),
                },
                Stanza::Presence {
                    kind: Some("probe".into()),
                    from: Some("owner@rig.test".into()),
                    to: "local@bot.rig.test".into(),
                },
                Stanza::Iq {
                    kind: "get".into(),
                    id: Some("q1".into()),
                    from: Some("owner@rig.test/rig".into()),
                    to: "bot.rig.test".into(),
                    child: Some((NS_DISCO_INFO.into(), "query".into())),
                },
                Stanza::Iq {
                    kind: "get".into(),
                    id: Some("q2".into()),
                    from: Some("owner@rig.test/rig".into()),
                    to: "jane.doe@bot.rig.test".into(),
                    child: Some(("vcard-temp".into(), "vCard".into())),
                },
                Stanza::Error,
                Stanza::Error,
            ]
        );
    }

    #[test]
    fn a_message_without_a_type_is_normal() {
        let read = stanzas("<message from='o@rig.test/r' to='bot.rig.test'><body>x</body></message><foo xmlns='bar'/>");
        assert!(matches!(&read[0], Stanza::Message { kind, .. } if kind == "normal"));
        assert_eq!(read[1], Stanza::Other("foo".into()));
    }
}
