//! An avatar's profile: from the `AgentProfile` capability where the
//! simulator offers it (on a worker thread, as name lookups are), from
//! `AvatarPropertiesReply` otherwise, and the `profile` command's text.

use crate::caps;
use crate::escape::escape;
use crate::names::Names;
use crate::net::Event;
use crate::xml::{self, Value};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(10);
/// The bit of `AvatarPropertiesReply`'s flags that says the avatar is
/// online (`AVATAR_ONLINE`).
pub const FLAG_ONLINE: u32 = 0x10;

/// What the grid tells of an avatar's profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: Uuid,
    /// The grid says it is online.
    pub online: bool,
    /// When the account was made, as `YYYY-MM-DD` where it can be read as
    /// a day, and as the grid wrote it otherwise.
    pub born: Option<String>,
    pub partner: Option<Uuid>,
    pub about: String,
    pub first_life: String,
}

/// The day an account was made as the grid wrote it: an LLSD date
/// (`2010-03-14T00:00:00Z`) or `M/D/YYYY` become `YYYY-MM-DD`, anything
/// else stays as it is. `None` when there is nothing.
fn born(given: &str) -> Option<String> {
    let given = given.trim();
    if given.is_empty() {
        return None;
    }
    let day = |y: &str, m: &str, d: &str| -> Option<String> {
        let (y, m, d) = (y.parse::<u32>().ok()?, m.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
        ((1..=12).contains(&m) && (1..=31).contains(&d) && (1000..=9999).contains(&y)).then(|| format!("{y:04}-{m:02}-{d:02}"))
    };
    let iso = given.get(..10).filter(|_| given.len() == 10 || given[10..].starts_with('T'));
    let read = match iso.map(|date| date.split('-').collect::<Vec<_>>()).as_deref() {
        Some([y, m, d]) if y.len() == 4 => day(y, m, d),
        _ => match given.split('/').collect::<Vec<_>>()[..] {
            [m, d, y] if y.len() == 4 => day(y, m, d),
            _ => None,
        },
    };
    Some(read.unwrap_or_else(|| given.to_string()))
}

fn partner(id: Uuid) -> Option<Uuid> {
    (!id.is_nil()).then_some(id)
}

/// Reads the `AgentProfile` reply for the avatar `asked`, with the keys
/// the viewer reads. A reply without `id`, or about another avatar, is
/// an avatar the grid does not know.
pub fn from_llsd(body: &str, asked: Uuid) -> Result<Profile, String> {
    let reply = xml::parse_llsd(body).map_err(|e| format!("unreadable profile of {asked}: {e}"))?;
    let id = reply.get("id").and_then(Value::as_uuid);
    if id != Some(asked) {
        return Err(format!("the grid knows no avatar {asked}"));
    }
    let flag = |key| matches!(reply.get(key), Some(Value::Bool(true)));
    let text = |key| reply.str_field(key).unwrap_or("").to_string();
    Ok(Profile {
        id: asked,
        online: flag("online"),
        born: match flag("hide_age") {
            true => None,
            false => born(&text("member_since")),
        },
        partner: reply.get("partner_id").and_then(Value::as_uuid).and_then(partner),
        about: text("sl_about_text"),
        first_life: text("fl_about_text"),
    })
}

/// Reads an `AvatarPropertiesReply`.
pub fn from_udp(avatar: Uuid, partner_id: Uuid, about: &str, first_life: &str, born_on: &str, flags: u32) -> Profile {
    Profile {
        id: avatar,
        online: flags & FLAG_ONLINE != 0,
        born: born(born_on),
        partner: partner(partner_id),
        about: about.to_string(),
        first_life: first_life.to_string(),
    }
}

/// The answer to `profile`: `name: value` lines. `names` as far as the
/// name service gave them; `friend` and `sees_online` from the friend
/// list.
pub fn text(profile: &Profile, names: Option<&Names>, friend: bool, sees_online: bool) -> String {
    let mut lines = vec![format!("id: {}", profile.id)];
    if let Some(names) = names {
        lines.push(format!("name: {}", escape(&names.legacy)));
        lines.push(format!("display name: {}", escape(&names.display)));
    }
    let online = match (profile.online, sees_online) {
        (true, _) => "yes",
        (false, true) => "no",
        (false, false) => "unknown",
    };
    lines.push(format!("online: {online}"));
    lines.push(format!("friend: {}", if friend { "yes" } else { "no" }));
    if let Some(born) = &profile.born {
        lines.push(format!("born: {}", escape(born)));
    }
    if let Some(partner) = profile.partner {
        lines.push(format!("partner: {partner}"));
    }
    for (key, value) in [("about", &profile.about), ("first life", &profile.first_life)] {
        if !value.is_empty() {
            lines.push(format!("{key}: {}", escape(value)));
        }
    }
    lines.join("\n")
}

/// A request to the worker: the profile of `id` from the capability at
/// `url`.
#[derive(Debug, PartialEq)]
pub struct Fetch {
    pub url: String,
    pub id: Uuid,
}

fn fetch(agent: &ureq::Agent, Fetch { url, id }: &Fetch) -> Result<Profile, String> {
    let url = format!("{}/{id}", url.trim_end_matches('/'));
    let failed = |reason: String| format!("the profile of {id} could not be read: {reason}");
    let mut response = agent
        .get(&url)
        .header("Accept", "application/llsd+xml")
        .call()
        .map_err(|e| failed(e.to_string()))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(failed(format!("HTTP {status}")));
    }
    let body = response.body_mut().read_to_string().map_err(|e| failed(e.to_string()))?;
    from_llsd(&body, *id)
}

/// Worker thread body: answers every fetch with `Event::Profile`.
fn run(fetches: Receiver<Fetch>, events: Sender<Event>) {
    let agent = caps::http_agent(TIMEOUT);
    for request in fetches {
        let result = fetch(&agent, &request);
        if events.send(Event::Profile { id: request.id, result }).is_err() {
            return;
        }
    }
}

/// Starts the worker; fetches sent to the returned channel are answered
/// on `events`.
pub fn spawn(events: Sender<Event>) -> Sender<Fetch> {
    let (sender, fetches) = mpsc::channel();
    thread::spawn(move || run(fetches, events));
    sender
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    pub fn jane() -> Uuid {
        Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap()
    }

    pub fn john() -> Uuid {
        Uuid::parse_str("22222222-3333-4444-5555-666666666666").unwrap()
    }

    pub fn jane_names() -> Names {
        Names {
            legacy: "Jane Doe".to_string(),
            display: "JD".to_string(),
        }
    }

    /// Jane Doe's profile as `AgentProfile` gives it.
    pub fn llsd(id: Uuid, online: bool) -> String {
        format!(
            "<llsd><map>\
               <key>id</key><uuid>{id}</uuid>\
               <key>online</key><boolean>{}</boolean>\
               <key>member_since</key><date>2010-03-14T00:00:00Z</date>\
               <key>hide_age</key><boolean>0</boolean>\
               <key>partner_id</key><uuid>{}</uuid>\
               <key>sl_about_text</key><string>Builder.</string>\
               <key>fl_about_text</key><string>Not telling.</string>\
               <key>sl_image_id</key><uuid>00000000-0000-0000-0000-000000000000</uuid>\
             </map></llsd>",
            u8::from(online),
            john()
        )
    }

    /// The README's example.
    pub fn full() -> Profile {
        from_udp(jane(), john(), "Builder.", "Not telling.", "3/14/2010", FLAG_ONLINE)
    }

    #[test]
    fn both_ways_give_the_same_profile() {
        assert_eq!(from_llsd(&llsd(jane(), true), jane()), Ok(full()));
        let offline = from_udp(jane(), john(), "Builder.", "Not telling.", "3/14/2010", 0x01);
        assert_eq!(from_llsd(&llsd(jane(), false), jane()), Ok(offline));
    }

    #[test]
    fn the_lines_of_a_profile() {
        let text = text(&full(), Some(&jane_names()), true, true);
        assert_eq!(
            text,
            format!(
                "id: {}\nname: Jane Doe\ndisplay name: JD\nonline: yes\nfriend: yes\nborn: 2010-03-14\n\
                 partner: {}\nabout: Builder.\nfirst life: Not telling.",
                jane(),
                john()
            )
        );
        let stranger = super::text(&full(), Some(&jane_names()), false, false);
        assert!(stranger.contains("\nfriend: no\n"));
    }

    #[test]
    fn lines_that_are_left_out() {
        let bare = from_udp(jane(), Uuid::nil(), "", "", "", 0);
        assert_eq!(
            text(&bare, Some(&jane_names()), false, false),
            format!("id: {}\nname: Jane Doe\ndisplay name: JD\nonline: unknown\nfriend: no", jane())
        );
        // Without names.
        assert_eq!(
            text(&bare, None, false, false),
            format!("id: {}\nonline: unknown\nfriend: no", jane())
        );
        // Hidden age on the capability.
        let hidden = llsd(jane(), false).replace("<key>hide_age</key><boolean>0</boolean>", "<key>hide_age</key><boolean>1</boolean>");
        assert_eq!(from_llsd(&hidden, jane()).unwrap().born, None);
        // Keys that are missing are empty.
        let sparse = format!("<llsd><map><key>id</key><uuid>{}</uuid></map></llsd>", jane());
        assert_eq!(from_llsd(&sparse, jane()), Ok(bare));
    }

    #[test]
    fn values_stay_on_their_line() {
        let several = from_udp(jane(), Uuid::nil(), "Builder.\nAsk me about sculpts.", "a\tb\\", "", 0);
        let text = text(&several, None, false, false);
        assert!(text.contains("\nabout: Builder.\\nAsk me about sculpts.\n"), "{text}");
        assert!(text.ends_with("\nfirst life: a\\tb\\\\"), "{text}");
    }

    #[test]
    fn the_day_of_birth() {
        assert_eq!(born("3/14/2010").as_deref(), Some("2010-03-14"));
        assert_eq!(born("12/1/2007").as_deref(), Some("2007-12-01"));
        assert_eq!(born("2010-03-14T00:00:00Z").as_deref(), Some("2010-03-14"));
        assert_eq!(born("2010-03-14").as_deref(), Some("2010-03-14"));
        // What cannot be read as a day passes through.
        assert_eq!(born("long ago").as_deref(), Some("long ago"));
        assert_eq!(born("13/40/2010").as_deref(), Some("13/40/2010"));
        assert_eq!(born("  "), None);
    }

    #[test]
    fn online_status() {
        let online = full();
        let offline = Profile { online: false, ..full() };
        let line = |profile: &Profile, friend, sees| text(profile, None, friend, sees).lines().nth(1).unwrap().to_string();
        assert_eq!(line(&online, true, true), "online: yes");
        assert_eq!(line(&offline, true, true), "online: no");
        assert_eq!(line(&online, false, false), "online: yes");
        assert_eq!(line(&offline, false, false), "online: unknown");
        // A friend that hides its status.
        assert_eq!(line(&offline, true, false), "online: unknown");
    }

    #[test]
    fn a_reply_about_nobody_or_another_avatar_is_an_error() {
        let other = from_llsd(&llsd(john(), true), jane()).unwrap_err();
        assert!(other.contains(&jane().to_string()), "{other}");
        assert!(from_llsd("<llsd><map></map></llsd>", jane())
            .unwrap_err()
            .contains(&jane().to_string()));
        assert!(from_llsd("oops", jane()).unwrap_err().contains(&jane().to_string()));
    }

    /// Serves `response` to one HTTP request and returns the request line.
    fn serve_once(listener: TcpListener, response: String) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            stream.write_all(response.as_bytes()).unwrap();
            request_line
        })
    }

    fn fetched(status: &str, body: &str) -> (Result<Profile, String>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/CAPS/profile/", listener.local_addr().unwrap());
        let server = serve_once(
            listener,
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/llsd+xml\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );
        let (events, incoming) = mpsc::channel();
        let fetches = spawn(events);
        fetches.send(Fetch { url, id: jane() }).unwrap();
        let Ok(Event::Profile { id, result }) = incoming.recv_timeout(Duration::from_secs(10)) else {
            panic!("no Profile event")
        };
        assert_eq!(id, jane());
        (result, server.join().unwrap())
    }

    #[test]
    fn worker_reads_a_profile_over_http() {
        let (result, request_line) = fetched("200 OK", &llsd(jane(), true));
        assert_eq!(result, Ok(full()));
        assert!(
            request_line.starts_with(&format!("GET /CAPS/profile/{} ", jane())),
            "{request_line}"
        );
    }

    #[test]
    fn worker_reports_an_error_status() {
        let (result, _) = fetched("404 Not Found", "");
        let error = result.unwrap_err();
        assert!(error.contains("HTTP 404") && error.contains(&jane().to_string()), "{error}");
    }

    #[test]
    fn worker_refuses_a_reply_about_another_avatar() {
        let (result, _) = fetched("200 OK", &llsd(john(), true));
        assert!(result.unwrap_err().contains(&jane().to_string()));
    }

    #[test]
    fn the_readme_shows_this_profile() {
        let text = text(&full(), Some(&jane_names()), true, true);
        assert!(include_str!("../README.md").contains(&format!("```\n{text}\n```")));
    }
}
