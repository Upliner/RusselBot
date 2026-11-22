//! The `scan` command's own part: its arguments, which of the objects
//! around the avatar they select, and the listing. No session and no clock:
//! what the simulator has described comes in as `Sighting`s.

use crate::escape::escape;
use crate::names;
use crate::udp::messages::{Properties, FLAG_PHANTOM, FLAG_PHYSICS, FLAG_SCRIPTED, FLAG_TEMPORARY, FLAG_TOUCH};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The furthest a `scan` reaches, in metres: the limit of `scan_distance`.
pub const MAX_DISTANCE: f32 = 1024.0;

/// A prim the simulator has described, as a `scan` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    pub object: Uuid,
    /// Region-local id, which is what the simulator is asked about it by.
    pub local_id: u32,
    /// How far it is from the avatar, in metres: its linkset's root for a
    /// child prim, the wearer for what an avatar wears.
    pub distance: f32,
    /// Where the distance is measured to, in the region.
    pub position: [f32; 3],
    /// Its `UpdateFlags`.
    pub flags: u32,
    /// An avatar wears it.
    pub attachment: bool,
    /// It is not the root of its linkset.
    pub child: bool,
    /// What it is called and whose it is, once the simulator has said.
    pub properties: Option<Properties>,
}

/// The flags a listing names, in the order it names them.
const FLAG_WORDS: [(u32, &str); 5] = [
    (FLAG_SCRIPTED, "scripted"),
    (FLAG_TOUCH, "touch"),
    (FLAG_PHYSICS, "physical"),
    (FLAG_TEMPORARY, "temporary"),
    (FLAG_PHANTOM, "phantom"),
];

/// Whose objects a `scan` is to list.
#[derive(Debug, Clone, PartialEq)]
pub enum Whose {
    /// An avatar's or a group's, by UUID.
    Id(Uuid),
    /// An avatar's, by its full name in lower case, `first last`.
    Name(String),
}

/// What a `scan` command asks for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    /// How far to list, in metres; as far as the object scan reaches when
    /// left out.
    pub distance: Option<f32>,
    /// Text the name has to contain, in lower case.
    pub name: Option<String>,
    /// Text the description has to contain, in lower case.
    pub description: Option<String>,
    pub owner: Option<Whose>,
    /// The `UpdateFlags` bits a prim has to have, all of them.
    pub flags: u32,
    /// Also list what avatars wear.
    pub attachments: bool,
    /// Also list the child prims of linksets.
    pub children: bool,
    /// Write each prim's position into its line.
    pub position: bool,
}

/// What a `scan` command is: a scan of the objects around the avatar, or
/// of the avatars in the region, with the distance to list them within.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Objects(Query),
    Avatars(Option<f32>),
}

impl Command {
    /// Reads the arguments of `scan`: `avatars` first selects the scan of
    /// avatars, `objects` first is the scan of objects as without it.
    pub fn parse(args: &[String]) -> Result<Command, String> {
        match args.first().map(String::as_str) {
            Some("avatars") => {
                let rest = &args[1..];
                if let Some(option) = rest.iter().find(|arg| arg.starts_with('-')) {
                    return Err(format!("`scan avatars` has no option `{option}`"));
                }
                match rest {
                    [] => Ok(Command::Avatars(None)),
                    [given] => Ok(Command::Avatars(Some(distance(given)?))),
                    _ => Err(format!("`scan avatars` takes one distance, not `{}`", rest.join("` and `"))),
                }
            }
            Some("objects") => Query::parse(&args[1..]).map(Command::Objects),
            _ => Query::parse(args).map(Command::Objects),
        }
    }
}

/// Reads a distance: a number of metres from 1 to 1024, or `shout`,
/// `chat` or `whisper`, which stand for 100, 20 and 10 metres.
pub fn distance(given: &str) -> Result<f32, String> {
    let metres = match given {
        "shout" => Some(100.0),
        "chat" => Some(20.0),
        "whisper" => Some(10.0),
        number => number.parse::<f32>().ok().filter(|metres| (1.0..=MAX_DISTANCE).contains(metres)),
    };
    metres.ok_or_else(|| {
        format!("the distance is a number of metres from 1 to {MAX_DISTANCE} or `shout`, `chat` or `whisper`, not `{given}`")
    })
}

/// The owner of an object and whether that is a group: a group's object
/// has no owner, or the group as its owner.
pub fn owner(properties: &Properties) -> (Uuid, bool) {
    if properties.owner.is_nil() || properties.owner == properties.group {
        (properties.group, true)
    } else {
        (properties.owner, false)
    }
}

impl Query {
    /// Reads the arguments of `scan`: a distance and the filters, in any
    /// order.
    pub fn parse(args: &[String]) -> Result<Query, String> {
        let mut query = Query::default();
        let mut distance: Option<&str> = None;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = |what: &str| args.next().ok_or_else(|| format!("`{arg}` takes {what}"));
            match arg.as_str() {
                "-n" | "--name" => query.name = Some(value("a text")?.to_lowercase()),
                "-d" | "--desc" => query.description = Some(value("a text")?.to_lowercase()),
                "-o" | "--owner" => query.owner = Some(whose(value("an avatar or a group's UUID")?)?),
                "-s" | "--scripted" => query.flags |= FLAG_SCRIPTED,
                "-t" | "--touch" => query.flags |= FLAG_TOUCH,
                "-p" | "--physical" => query.flags |= FLAG_PHYSICS,
                "--temporary" => query.flags |= FLAG_TEMPORARY,
                "--phantom" => query.flags |= FLAG_PHANTOM,
                "-a" | "--attachments" => query.attachments = true,
                "-c" | "--children" => query.children = true,
                "--position" => query.position = true,
                option if option.starts_with('-') => return Err(format!("`scan` has no option `{option}`")),
                given => match distance.replace(given) {
                    None => {}
                    Some(first) => return Err(format!("`scan` takes one distance, not `{first}` and `{given}`")),
                },
            }
        }
        if let Some(given) = distance {
            query.distance = Some(self::distance(given)?);
        }
        Ok(query)
    }

    /// Whether `sighting` is within `distance` metres and meets the flag
    /// filters and the switches: what can be told without its properties,
    /// and so what the simulator is asked for the properties of.
    pub fn reachable(&self, sighting: &Sighting, distance: f32) -> bool {
        sighting.distance <= distance
            && sighting.flags & self.flags == self.flags
            && (self.attachments || !sighting.attachment)
            && (self.children || !sighting.child)
    }

    /// Whether an object with these properties meets the filters on its
    /// name and its description, and the one on its owner when that names
    /// a UUID: what can be told without the owners' names.
    fn named(&self, properties: &Properties) -> bool {
        let contains = |text: &str, wanted: &Option<String>| wanted.as_ref().is_none_or(|wanted| text.to_lowercase().contains(wanted));
        let owned = match &self.owner {
            Some(Whose::Id(id)) => owner(properties).0 == *id,
            _ => true,
        };
        owned && contains(&properties.name, &self.name) && contains(&properties.description, &self.description)
    }

    /// Whether `sighting` meets the filters on its name, its description
    /// and its owner. `owners` are the avatars' names as far as the name
    /// service gave them. What is not known matches no filter.
    pub fn matches(&self, sighting: &Sighting, owners: &HashMap<Uuid, Option<String>>) -> bool {
        let Some(properties) = &sighting.properties else {
            return self.name.is_none() && self.description.is_none() && self.owner.is_none();
        };
        let owned = match &self.owner {
            Some(Whose::Name(name)) => {
                let (owner, group) = owner(properties);
                let known = owners.get(&owner).and_then(|name| name.as_deref());
                !group && known.is_some_and(|known| known.to_lowercase() == *name)
            }
            _ => true,
        };
        owned && self.named(properties)
    }

    /// The avatar whose name the listing needs on account of `sighting`:
    /// its owner, when the object meets every filter that can be told
    /// without that name. Nobody for a group's object.
    pub fn owner_wanted(&self, sighting: &Sighting) -> Option<Uuid> {
        let properties = sighting.properties.as_ref().filter(|properties| self.named(properties))?;
        let (owner, group) = owner(properties);
        (!group).then_some(owner)
    }
}

/// Reads whose objects `-o` asks for: an avatar as `im` takes one, or a
/// UUID, which may be a group's.
fn whose(given: &str) -> Result<Whose, String> {
    let given = given.trim();
    if let Ok(id) = Uuid::parse_str(given) {
        return Ok(Whose::Id(id));
    }
    names::full_name(given)
        .map(Whose::Name)
        .ok_or_else(|| format!("\"{given}\" is not an avatar name (`First Last`, `first.last` or one word) or a UUID"))
}

/// The flags field of a line: the words that apply, or `-`.
fn flag_words(sighting: &Sighting) -> String {
    let set = FLAG_WORDS
        .iter()
        .filter(|(bit, _)| sighting.flags & bit != 0)
        .map(|(_, word)| *word);
    let placed = [(sighting.attachment, "attachment"), (sighting.child, "child")];
    let words: Vec<&str> = set.chain(placed.iter().filter(|(is, _)| *is).map(|(_, word)| *word)).collect();
    if words.is_empty() {
        "-".to_string()
    } else {
        words.join(",")
    }
}

/// The listing of `sightings`: one line each, nearest first, of the UUID,
/// the distance, the flags, the owner's UUID and name, the name and the
/// description, separated by tabs, with `?` for what is not known and for
/// an owner in `hidden`, unless it is a group. `None` when there is
/// nothing to list.
#[cfg(test)]
pub fn listing(sightings: Vec<Sighting>, owners: &HashMap<Uuid, Option<String>>, hidden: &HashSet<Uuid>) -> Option<String> {
    listing_with(sightings, owners, hidden, false)
}

/// `listing`, and with `position` each line has the prim's position in the
/// region, `x,y,z` in whole metres, as a field of its own after the
/// distance.
pub fn listing_with(
    mut sightings: Vec<Sighting>,
    owners: &HashMap<Uuid, Option<String>>,
    hidden: &HashSet<Uuid>,
    position: bool,
) -> Option<String> {
    sightings.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.object.cmp(&b.object)));
    let lines: Vec<String> = sightings
        .iter()
        .map(|sighting| {
            let unknown = || "?".to_string();
            let known = match &sighting.properties {
                None => [unknown(), unknown(), unknown(), unknown()],
                Some(properties) => {
                    let (id, group) = owner(properties);
                    let named = match owners.get(&id) {
                        _ if group => "(group)".to_string(),
                        _ if hidden.contains(&id) => unknown(),
                        Some(Some(name)) => escape(name),
                        _ => unknown(),
                    };
                    let id = match !group && hidden.contains(&id) {
                        true => unknown(),
                        false => id.to_string(),
                    };
                    [id, named, escape(&properties.name), escape(&properties.description)]
                }
            };
            let [owner, owner_name, name, description] = known;
            let place = match position {
                true => {
                    let [x, y, z] = sighting.position.map(|v| v.round() as i64);
                    format!("{x},{y},{z}\t")
                }
                false => String::new(),
            };
            format!(
                "{}\t{:.1}\t{place}{}\t{owner}\t{owner_name}\t{name}\t{description}",
                sighting.object,
                sighting.distance,
                flag_words(sighting)
            )
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    fn parse(words: &[&str]) -> Result<Query, String> {
        Query::parse(&args(words))
    }

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn jane() -> Uuid {
        id(0x1A)
    }

    fn club() -> Uuid {
        id(0x6C)
    }

    /// Jane Doe's object.
    fn named(name: &str, description: &str) -> Option<Properties> {
        Some(Properties {
            object: id(1),
            owner: jane(),
            group: Uuid::nil(),
            name: name.to_string(),
            description: description.to_string(),
        })
    }

    fn sighting(n: u8, distance: f32, properties: Option<Properties>) -> Sighting {
        Sighting {
            object: id(n),
            local_id: u32::from(n),
            distance,
            position: [0.0; 3],
            flags: 0,
            attachment: false,
            child: false,
            properties,
        }
    }

    fn owners() -> HashMap<Uuid, Option<String>> {
        HashMap::from([(jane(), Some("Jane Doe".to_string())), (id(0x2B), Some("John Roe".to_string()))])
    }

    #[test]
    fn no_arguments_ask_for_everything_in_reach() {
        assert_eq!(parse(&[]), Ok(Query::default()));
    }

    #[test]
    fn a_distance_and_filters_come_in_any_order() {
        let query = parse(&["-s", "20", "-o", "Jane Doe", "-n", "Door"]).unwrap();
        let wanted = Query {
            distance: Some(20.0),
            name: Some("door".to_string()),
            owner: Some(Whose::Name("jane doe".to_string())),
            flags: FLAG_SCRIPTED,
            ..Query::default()
        };
        assert_eq!(query, wanted);
        assert_eq!(parse(&["-n", "Door", "-o", "jane.doe", "20", "-s"]), Ok(wanted));
        assert_eq!(parse(&["12.5"]).unwrap().distance, Some(12.5));
    }

    #[test]
    fn every_filter_has_a_long_spelling() {
        let short = parse(&["-n", "a b", "-d", "c", "-o", "jane", "-s", "-t", "-p", "-a", "-c"]).unwrap();
        let long = parse(&[
            "--name",
            "A B",
            "--desc",
            "C",
            "--owner",
            "Jane",
            "--scripted",
            "--touch",
            "--physical",
            "--attachments",
            "--children",
        ]);
        assert_eq!(long, Ok(short.clone()));
        assert_eq!(short.name.as_deref(), Some("a b"));
        assert_eq!(short.description.as_deref(), Some("c"));
        assert_eq!(short.owner, Some(Whose::Name("jane resident".to_string())));
        assert_eq!(short.flags, FLAG_SCRIPTED | FLAG_TOUCH | FLAG_PHYSICS);
        assert!(short.attachments && short.children);
        assert_eq!(parse(&["--temporary", "--phantom"]).unwrap().flags, FLAG_TEMPORARY | FLAG_PHANTOM);
    }

    #[test]
    fn the_documented_examples_parse() {
        // The README's, as a shell hands them over.
        let uuid = jane().to_string();
        let examples: [&[&str]; 5] = [
            &[],
            &["20", "-n", "bench"],
            &["-t"],
            &["96", "-o", "Jane Doe"],
            &["-a", "-c", "-o", &uuid],
        ];
        for example in examples {
            assert!(parse(example).is_ok(), "{example:?}");
        }
        let commands: [&[&str]; 6] = [
            &["objects", "chat", "-t"],
            &["shout"],
            &["avatars"],
            &["avatars", "chat"],
            &["avatars", "50"],
            &["objects"],
        ];
        for example in commands {
            assert!(Command::parse(&args(example)).is_ok(), "{example:?}");
        }
    }

    #[test]
    fn an_owner_is_a_name_or_a_uuid() {
        let uuid = jane().to_string();
        assert_eq!(parse(&["-o", &uuid]).unwrap().owner, Some(Whose::Id(jane())));
        assert!(parse(&["-o", "one two three"]).unwrap_err().contains("\"one two three\""));
    }

    #[test]
    fn unknown_options_are_refused_by_name() {
        assert!(parse(&["-x"]).unwrap_err().contains("`-x`"));
        assert!(parse(&["--scripts"]).unwrap_err().contains("`--scripts`"));
        assert!(parse(&["-5"]).unwrap_err().contains("`-5`"));
    }

    #[test]
    fn a_filter_without_its_text_is_refused() {
        assert_eq!(parse(&["-n"]), Err("`-n` takes a text".to_string()));
        assert_eq!(parse(&["10", "--desc"]), Err("`--desc` takes a text".to_string()));
        assert!(parse(&["-o"]).unwrap_err().starts_with("`-o` takes"));
    }

    #[test]
    fn the_distance_is_one_number_within_bounds() {
        for given in ["0", "2000", "far", "0.5", "nan", "inf"] {
            let error = parse(&[given]).unwrap_err();
            assert!(
                error.contains("from 1 to 1024 or `shout`, `chat` or `whisper`") && error.contains(given),
                "{error}"
            );
        }
        assert_eq!(parse(&["1"]).unwrap().distance, Some(1.0));
        assert_eq!(parse(&["1024"]).unwrap().distance, Some(1024.0));
        assert!(parse(&["10", "20"]).unwrap_err().contains("one distance"));
    }

    #[test]
    fn the_distance_may_be_a_word() {
        assert_eq!(parse(&["chat"]).unwrap().distance, Some(20.0));
        assert_eq!(parse(&["shout"]).unwrap().distance, Some(100.0));
        assert_eq!(parse(&["-t", "whisper"]).unwrap(), parse(&["10", "-t"]).unwrap());
        // A word and a number are two distances.
        assert!(parse(&["chat", "20"]).unwrap_err().contains("one distance"));
        // Only in lower case.
        for given in ["Chat", "far", "SHOUT"] {
            let error = parse(&[given]).unwrap_err();
            assert!(error.contains("`shout`, `chat` or `whisper`") && error.contains(given), "{error}");
        }
        assert_eq!(distance("whisper"), Ok(10.0));
        assert_eq!(distance("12.5"), Ok(12.5));
    }

    fn command(words: &[&str]) -> Result<Command, String> {
        Command::parse(&args(words))
    }

    #[test]
    fn the_word_objects_changes_nothing() {
        for words in [&[][..], &["20", "-n", "bench"][..], &["-t", "whisper"][..]] {
            let with: Vec<&str> = std::iter::once("objects").chain(words.iter().copied()).collect();
            assert_eq!(command(&with), command(words));
            assert_eq!(command(words), parse(words).map(Command::Objects));
        }
        assert_eq!(
            command(&["objects", "-t", "whisper"]),
            Ok(Command::Objects(Query {
                distance: Some(10.0),
                flags: FLAG_TOUCH,
                ..Query::default()
            }))
        );
        // Anywhere else it is read as a distance, and refused.
        let error = command(&["20", "objects"]).unwrap_err();
        assert!(error.contains("one distance"), "{error}");
        assert!(command(&["objects", "objects"]).unwrap_err().contains("`objects`"));
    }

    #[test]
    fn scan_avatars_takes_one_distance_at_most() {
        assert_eq!(command(&["avatars"]), Ok(Command::Avatars(None)));
        assert_eq!(command(&["avatars", "chat"]), Ok(Command::Avatars(Some(20.0))));
        assert_eq!(command(&["avatars", "12.5"]), Ok(Command::Avatars(Some(12.5))));
        assert_eq!(command(&["avatars", "shout"]), Ok(Command::Avatars(Some(100.0))));
        // The object scan's filters are no part of it.
        assert!(command(&["avatars", "-n", "jane"]).unwrap_err().contains("`-n`"));
        assert!(command(&["avatars", "-t"]).unwrap_err().contains("`-t`"));
        for given in ["0", "2000", "near", "Chat"] {
            let error = command(&["avatars", given]).unwrap_err();
            assert!(
                error.contains("from 1 to 1024 or `shout`, `chat` or `whisper`") && error.contains(given),
                "{error}"
            );
        }
        assert!(command(&["avatars", "chat", "30"]).is_err());
        assert!(command(&["avatars", "10", "20"]).is_err());
        // Not the first argument: the object scan's distance, and refused.
        assert!(command(&["20", "avatars"]).is_err());
    }

    #[test]
    fn reach_is_distance_flags_and_switches() {
        let near = sighting(1, 9.9, None);
        let query = Query::default();
        assert!(query.reachable(&near, 10.0));
        assert!(!query.reachable(&near, 9.0));

        let scripted = Sighting {
            flags: FLAG_SCRIPTED | FLAG_PHANTOM,
            ..near.clone()
        };
        let wants = |flags| Query { flags, ..Query::default() };
        assert!(wants(FLAG_SCRIPTED).reachable(&scripted, 10.0));
        assert!(!wants(FLAG_SCRIPTED).reachable(&near, 10.0));
        // Two flags: both have to be there.
        assert!(!wants(FLAG_SCRIPTED | FLAG_PHYSICS).reachable(&scripted, 10.0));
        assert!(wants(FLAG_SCRIPTED | FLAG_PHANTOM).reachable(&scripted, 10.0));

        let hat = Sighting {
            attachment: true,
            ..near.clone()
        };
        let cushion = Sighting {
            child: true,
            ..near.clone()
        };
        let strap = Sighting {
            attachment: true,
            child: true,
            ..near.clone()
        };
        let with = |attachments, children| Query {
            attachments,
            children,
            ..Query::default()
        };
        assert!(!query.reachable(&hat, 10.0) && !query.reachable(&cushion, 10.0));
        assert!(with(true, false).reachable(&hat, 10.0) && !with(true, false).reachable(&strap, 10.0));
        assert!(with(false, true).reachable(&cushion, 10.0) && !with(false, true).reachable(&strap, 10.0));
        assert!(with(true, true).reachable(&strap, 10.0));
    }

    #[test]
    fn text_filters_look_for_a_part_whatever_the_case() {
        let by_name = |text: &str| parse(&["-n", text]).unwrap();
        let bench = sighting(1, 1.0, named("Park Bench", "Sit here"));
        let workbench = sighting(2, 1.0, named("Workbench", ""));
        let lamp = sighting(3, 1.0, named("Lamp", "Teleporter to the shop"));
        let listed = |query: &Query| -> Vec<bool> { [&bench, &workbench, &lamp].map(|s| query.matches(s, &owners())).to_vec() };
        assert_eq!(listed(&by_name("bench")), [true, true, false]);
        assert_eq!(listed(&by_name("park bench")), [true, false, false]);
        assert_eq!(listed(&parse(&["-d", "teleport"]).unwrap()), [false, false, true]);
        assert_eq!(listed(&parse(&["-d", "SIT", "-n", "bench"]).unwrap()), [true, false, false]);
        // What the simulator has not named matches no text.
        let unnamed = sighting(4, 1.0, None);
        assert!(!by_name("bench").matches(&unnamed, &owners()));
        assert!(!parse(&["-d", "x"]).unwrap().matches(&unnamed, &owners()));
        assert!(Query::default().matches(&unnamed, &owners()));
    }

    #[test]
    fn the_owner_filter_takes_names_avatars_and_groups() {
        let janes = sighting(1, 1.0, named("Bench", ""));
        let johns = Sighting {
            properties: Some(Properties {
                owner: id(0x2B),
                ..named("Lamp", "").unwrap()
            }),
            ..sighting(2, 1.0, None)
        };
        // Second Life names no owner for a group's object, OpenSimulator
        // the group itself.
        let deeded = |owner| Sighting {
            properties: Some(Properties {
                owner,
                group: club(),
                ..named("Sign", "").unwrap()
            }),
            ..sighting(3, 1.0, None)
        };
        let all = [janes, johns, deeded(Uuid::nil()), deeded(club())];
        let listed = |words: &[&str], owners: &HashMap<Uuid, Option<String>>| -> Vec<bool> {
            let query = parse(words).unwrap();
            all.iter().map(|s| query.matches(s, owners)).collect()
        };
        assert_eq!(listed(&["-o", "jane doe"], &owners()), [true, false, false, false]);
        assert_eq!(listed(&["-o", "Jane.Doe"], &owners()), [true, false, false, false]);
        assert_eq!(listed(&["-o", "Nobody Here"], &owners()), [false; 4]);
        // By UUID the name service is not needed.
        let nobody = HashMap::new();
        assert_eq!(listed(&["-o", &jane().to_string()], &nobody), [true, false, false, false]);
        assert_eq!(listed(&["-o", &club().to_string()], &nobody), [false, false, true, true]);
        // An owner whose name is not known matches no name.
        assert_eq!(listed(&["-o", "jane doe"], &nobody), [false; 4]);
        let closed = HashMap::from([(jane(), None)]);
        assert_eq!(listed(&["-o", "jane doe"], &closed), [false; 4]);
        // Nor does an object whose owner the simulator has not told.
        let unknown = sighting(5, 1.0, None);
        assert!(!parse(&["-o", &jane().to_string()]).unwrap().matches(&unknown, &owners()));
    }

    #[test]
    fn names_are_wanted_of_the_owners_of_what_is_listed() {
        let bench = sighting(1, 1.0, named("Bench", ""));
        let johns = Sighting {
            properties: Some(Properties {
                owner: id(0x2B),
                ..named("Lamp", "").unwrap()
            }),
            ..sighting(2, 1.0, None)
        };
        let sign = Sighting {
            properties: Some(Properties {
                owner: Uuid::nil(),
                group: club(),
                ..named("Sign", "").unwrap()
            }),
            ..sighting(3, 1.0, None)
        };
        let unnamed = sighting(4, 1.0, None);
        let wanted = |words: &[&str]| -> Vec<Option<Uuid>> {
            let query = parse(words).unwrap();
            [&bench, &johns, &sign, &unnamed].map(|s| query.owner_wanted(s)).to_vec()
        };
        assert_eq!(wanted(&[]), [Some(jane()), Some(id(0x2B)), None, None]);
        // Not of what the name filter leaves out.
        assert_eq!(wanted(&["-n", "lamp"]), [None, Some(id(0x2B)), None, None]);
        assert_eq!(wanted(&["-o", &jane().to_string()]), [Some(jane()), None, None, None]);
        // Whose it is by name can only be told with the names.
        assert_eq!(wanted(&["-o", "jane doe"]), [Some(jane()), Some(id(0x2B)), None, None]);
    }

    #[test]
    fn a_line_has_seven_fields() {
        let bench = Sighting {
            flags: FLAG_SCRIPTED | FLAG_TOUCH,
            ..sighting(1, 3.24, named("Bench", "Sit here"))
        };
        let line = listing(vec![bench], &owners(), &HashSet::new()).unwrap();
        assert_eq!(
            line,
            format!("{}\t3.2\tscripted,touch\t{}\tJane Doe\tBench\tSit here", id(1), jane())
        );
    }

    #[test]
    fn a_line_with_its_position() {
        let bench = Sighting {
            flags: FLAG_SCRIPTED | FLAG_TOUCH,
            position: [103.24, 100.0, 25.0],
            ..sighting(1, 3.24, named("Bench", "Sit here"))
        };
        let line = listing_with(vec![bench.clone()], &owners(), &HashSet::new(), true).unwrap();
        assert_eq!(
            line,
            format!("{}\t3.2\t103,100,25\tscripted,touch\t{}\tJane Doe\tBench\tSit here", id(1), jane())
        );
        // Without the switch the line is what it always was.
        assert_eq!(
            listing_with(vec![bench.clone()], &owners(), &HashSet::new(), false),
            listing(vec![bench], &owners(), &HashSet::new())
        );
        // A child prim is where its linkset's root is: the position the
        // sighting carries, whatever the prim.
        let cushion = Sighting {
            child: true,
            position: [50.4, 59.6, 22.0],
            ..sighting(2, 4.0, named("Cushion", ""))
        };
        let line = listing_with(vec![cushion], &owners(), &HashSet::new(), true).unwrap();
        assert_eq!(line.split('\t').nth(2), Some("50,60,22"));
        assert_eq!(line.split('\t').count(), 8);
    }

    #[test]
    fn the_position_switch_filters_nothing_and_goes_anywhere() {
        let plain = parse(&["20", "-n", "bench"]).unwrap();
        for words in [
            &["--position", "20", "-n", "bench"][..],
            &["20", "--position", "-n", "bench"],
            &["20", "-n", "bench", "--position"],
        ] {
            let query = parse(words).unwrap();
            assert!(query.position);
            assert_eq!(Query { position: false, ..query }, plain);
        }
        assert!(!plain.position);
        let query = parse(&["-t", "--position", "whisper"]).unwrap();
        assert_eq!((query.flags, query.distance, query.position), (FLAG_TOUCH, Some(10.0), true));
        // The same prims in the same order, each line a field longer.
        let (near, far) = (sighting(1, 2.0, named("Bench", "")), sighting(2, 9.0, named("Bench two", "")));
        let with = listing_with(vec![far.clone(), near.clone()], &owners(), &HashSet::new(), true).unwrap();
        let without = listing(vec![far, near], &owners(), &HashSet::new()).unwrap();
        let ids = |text: &str| -> Vec<String> { text.lines().map(|line| line.split('\t').next().unwrap().to_string()).collect() };
        assert_eq!(ids(&with), ids(&without));
        assert!(with.lines().all(|line| line.split('\t').count() == 8));
        assert!(without.lines().all(|line| line.split('\t').count() == 7));
        // Not for avatars.
        let refused = Command::parse(&args(&["avatars", "--position"])).unwrap_err();
        assert!(refused.contains("--position"), "{refused}");
    }

    #[test]
    fn a_groups_object_names_the_group() {
        for owner in [Uuid::nil(), club()] {
            let sign = Sighting {
                properties: Some(Properties {
                    owner,
                    group: club(),
                    ..named("Sign", "").unwrap()
                }),
                ..sighting(1, 2.0, None)
            };
            let line = listing(vec![sign], &owners(), &HashSet::new()).unwrap();
            // No description: the line ends with the tab after the name.
            assert_eq!(line, format!("{}\t2.0\t-\t{}\t(group)\tSign\t", id(1), club()));
        }
    }

    #[test]
    fn what_is_not_known_is_a_question_mark() {
        let unanswered = sighting(1, 2.0, None);
        assert_eq!(
            listing(vec![unanswered], &owners(), &HashSet::new()).unwrap(),
            format!("{}\t2.0\t-\t?\t?\t?\t?", id(1))
        );
        // The owner is known, the name service does not name it.
        let bench = sighting(1, 2.0, named("Bench", ""));
        for owners in [HashMap::new(), HashMap::from([(jane(), None)])] {
            let line = listing(vec![bench.clone()], &owners, &HashSet::new()).unwrap();
            assert_eq!(line, format!("{}\t2.0\t-\t{}\t?\tBench\t", id(1), jane()));
        }
    }

    #[test]
    fn lines_are_ordered_by_distance_then_uuid() {
        let far = sighting(1, 12.0, None);
        let near = sighting(3, 3.0, None);
        let middle = sighting(2, 7.0, None);
        let twin = sighting(4, 7.0, None);
        let text = listing(vec![far, twin, near, middle], &owners(), &HashSet::new()).unwrap();
        let order: Vec<String> = text.lines().map(|line| line.split('\t').next().unwrap().to_string()).collect();
        assert_eq!(order, [id(3), id(2), id(4), id(1)].map(|id| id.to_string()));
        assert_eq!(listing(Vec::new(), &owners(), &HashSet::new()), None);
    }

    #[test]
    fn names_stay_on_their_line_and_in_their_field() {
        let odd = sighting(1, 1.0, named("Tab\there", "two\nlines \\ slash\r"));
        let line = listing(vec![odd], &owners(), &HashSet::new()).unwrap();
        assert_eq!(line.lines().count(), 1);
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[5], "Tab\\there");
        assert_eq!(fields[6], "two\\nlines \\\\ slash\\r");
    }

    #[test]
    fn hidden_owners_are_question_marks() {
        let bench = sighting(1, 3.24, named("Bench", "Sit here"));
        let hidden = HashSet::from([jane()]);
        assert_eq!(
            listing(vec![bench.clone()], &owners(), &hidden).unwrap(),
            format!("{}\t3.2\t-\t?\t?\tBench\tSit here", id(1))
        );
        // Another owner is shown, and so is a group.
        let johns = Sighting {
            properties: Some(Properties {
                owner: id(0x2B),
                ..named("Lamp", "").unwrap()
            }),
            ..sighting(2, 1.0, None)
        };
        let sign = Sighting {
            properties: Some(Properties {
                owner: Uuid::nil(),
                group: jane(),
                ..named("Sign", "").unwrap()
            }),
            ..sighting(3, 2.0, None)
        };
        let text = listing(vec![johns, sign], &owners(), &hidden).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains(&format!("\t{}\tJohn Roe\tLamp", id(0x2B))), "{}", lines[0]);
        assert!(lines[1].contains(&format!("\t{}\t(group)\tSign", jane())), "{}", lines[1]);
    }

    #[test]
    fn flags_come_in_a_fixed_order() {
        let words = |flags, attachment, child| {
            flag_words(&Sighting {
                flags,
                attachment,
                child,
                ..sighting(1, 1.0, None)
            })
        };
        assert_eq!(words(0, false, false), "-");
        // Bits the listing does not name do not show.
        assert_eq!(words(1 << 3 | 1 << 20, false, false), "-");
        let all = FLAG_PHANTOM | FLAG_TEMPORARY | FLAG_PHYSICS | FLAG_TOUCH | FLAG_SCRIPTED;
        assert_eq!(words(all, true, true), "scripted,touch,physical,temporary,phantom,attachment,child");
        assert_eq!(words(FLAG_SCRIPTED, false, true), "scripted,child");
        assert_eq!(words(0, true, false), "attachment");
    }
}
