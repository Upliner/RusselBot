//! The placement the bot holds: an auto-return location and a sit state,
//! what the arguments of the `persist` command stand for, and what `last`
//! remembers. No I/O happens here; the live session is seen through `World`.

use crate::reconciler::same_region;
use crate::session::NOT_CONNECTED;
use crate::udp::messages::Vector3;
use std::collections::VecDeque;
use std::fmt;
use uuid::Uuid;

/// How many objects the avatar sat on are remembered with their region.
const MAX_SEATS: usize = 64;

/// A region and coordinates inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub region: String,
    pub position: Vector3,
}

/// What the bot does about sitting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SitState {
    /// Keep the avatar seated on this object.
    Object(Uuid),
    /// Sit it on the ground once it has arrived.
    Ground,
    /// Stand it up once, then leave it.
    Stand,
    /// Leave sitting and standing alone.
    Unchanged,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Missing only while the config names no position and no login has
    /// reported one.
    pub location: Option<Location>,
    pub sit: SitState,
}

/// The location argument of `persist`, still to be resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum LocationArg {
    Cur,
    Cfg,
    Last,
    LastCmd,
    At(Location),
}

/// The sit state argument of `persist`, still to be resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SitArg {
    Cur,
    Cfg,
    Last,
    LastCmd,
    Is(SitState),
}

/// How the avatar sits right now, as far as the bot can tell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sitting {
    /// On the prim the simulator named.
    Object(Uuid),
    /// Attached to something the simulator did not name.
    Unnamed,
    /// Not on an object, and the bot last sat it on the ground.
    Ground,
    Standing,
}

/// The avatar's surroundings while it is in a region and not on its way
/// out of it.
pub trait World {
    /// Where the avatar is.
    fn here(&self) -> Location;
    fn sitting(&self) -> Sitting;
    /// The current region's name, if `object` has been learned in it.
    fn region_of(&self, object: Uuid) -> Option<String>;
}

/// A coordinate to the centimetre, without trailing zeros.
pub fn coordinate(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let [x, y, z] = self.position.map(coordinate);
        write!(f, "{}/{x}/{y}/{z}", self.region)
    }
}

impl fmt::Display for SitState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SitState::Object(object) => write!(f, "{object}"),
            SitState::Ground => f.write_str("ground"),
            SitState::Stand => f.write_str("stand"),
            SitState::Unchanged => f.write_str("unchanged"),
        }
    }
}

impl fmt::Display for Placement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.location {
            Some(location) => write!(f, "{location} {}", self.sit),
            None => write!(f, "unknown {}", self.sit),
        }
    }
}

/// Reads `<region>/<x>/<y>/<z>` or one of the special words.
pub fn parse_location(text: &str) -> Result<LocationArg, String> {
    let text = text.trim();
    match text.to_ascii_lowercase().as_str() {
        "cur" => return Ok(LocationArg::Cur),
        "cfg" => return Ok(LocationArg::Cfg),
        "last" => return Ok(LocationArg::Last),
        "last_cmd" => return Ok(LocationArg::LastCmd),
        _ => {}
    }
    let malformed = || format!("\"{text}\" is not a location: write <region>/<x>/<y>/<z>, or cur, cfg, last or last_cmd");
    let parts: Vec<&str> = text.split('/').collect();
    let [region, x, y, z] = parts[..] else {
        return Err(malformed());
    };
    let region = region.trim();
    let number = |part: &str| match part.trim().parse::<f32>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(malformed()),
    };
    if region.is_empty() {
        return Err(malformed());
    }
    Ok(LocationArg::At(Location {
        region: region.to_string(),
        position: [number(x)?, number(y)?, number(z)?],
    }))
}

/// Reads an object's UUID, `ground`, `stand`, `unchanged` or one of the
/// special words.
pub fn parse_sit_state(text: &str) -> Result<SitArg, String> {
    let text = text.trim();
    Ok(match text.to_ascii_lowercase().as_str() {
        "cur" => SitArg::Cur,
        "cfg" => SitArg::Cfg,
        "last" => SitArg::Last,
        "last_cmd" => SitArg::LastCmd,
        "ground" => SitArg::Is(SitState::Ground),
        "stand" => SitArg::Is(SitState::Stand),
        "unchanged" => SitArg::Is(SitState::Unchanged),
        _ => match Uuid::parse_str(text) {
            Ok(object) => SitArg::Is(SitState::Object(object)),
            Err(_) => {
                return Err(format!(
                    "\"{text}\" is not a sit state: write an object's UUID, ground, stand, unchanged, \
                     cur, cfg, last or last_cmd"
                ))
            }
        },
    })
}

/// What the bot holds and what it remembers about placements, for as long
/// as it runs.
#[derive(Debug, Clone)]
pub struct Book {
    /// The placement the config describes.
    config: Placement,
    /// Where the first login of this run put the avatar.
    first: Option<Location>,
    /// The last placement persisted, if any was.
    last: Option<Placement>,
    held: Placement,
    /// The objects the avatar was seated on, with the region it was in,
    /// newest first.
    seats: VecDeque<(Uuid, String)>,
}

impl Book {
    pub fn new(config: Placement) -> Book {
        Book {
            held: config.clone(),
            config,
            first: None,
            last: None,
            seats: VecDeque::new(),
        }
    }

    /// The placement the bot works towards in holding mode, and the one
    /// `home` returns to.
    pub fn held(&self) -> &Placement {
        &self.held
    }

    /// The placement the config describes; its location is the first
    /// location when the config names none.
    pub fn config(&self) -> Placement {
        Placement {
            location: self.config.location.clone().or_else(|| self.first.clone()),
            sit: self.config.sit,
        }
    }

    fn last(&self) -> Placement {
        self.last.clone().unwrap_or_else(|| self.config())
    }

    /// The simulator reported where the avatar is after a login: the first
    /// such report fills every location that waited for it. Returns whether
    /// the held placement changed.
    pub fn first_location(&mut self, location: Location) -> bool {
        if self.first.is_some() {
            return false;
        }
        if let Some(Placement { location: slot @ None, .. }) = &mut self.last {
            *slot = Some(location.clone());
        }
        let changed = self.held.location.is_none();
        if changed {
            self.held.location = Some(location.clone());
        }
        self.first = Some(location);
        changed
    }

    /// The simulator confirmed the avatar seated on `object` in `region`.
    pub fn seated(&mut self, object: Uuid, region: &str) {
        self.seats.retain(|(known, _)| *known != object);
        self.seats.push_front((object, region.to_string()));
        self.seats.truncate(MAX_SEATS);
    }

    /// The region `object` is known to be in: the current one when it has
    /// been learned there, else the one the avatar last sat on it in.
    fn region_of(&self, object: Uuid, world: Option<&dyn World>) -> Option<String> {
        world.and_then(|world| world.region_of(object)).or_else(|| {
            self.seats
                .iter()
                .find(|(known, _)| *known == object)
                .map(|(_, region)| region.clone())
        })
    }

    /// What the two arguments of a persist stand for right now. `world` is
    /// the avatar's surroundings while it is in a region; `last_tp` and
    /// `last_sit` are what the last `tp` and `sit` commands named. Changes
    /// nothing.
    pub fn resolve(
        &self,
        location: &LocationArg,
        sit: SitArg,
        world: Option<&dyn World>,
        last_tp: Option<&Location>,
        last_sit: Option<SitState>,
    ) -> Result<Placement, String> {
        let location = match location {
            LocationArg::At(location) => Some(location.clone()),
            LocationArg::Cur => Some(world.ok_or(NOT_CONNECTED)?.here()),
            LocationArg::Cfg => self.config().location,
            LocationArg::Last => self.last().location,
            LocationArg::LastCmd => last_tp.cloned().or_else(|| self.last().location),
        };
        let sit = match sit {
            SitArg::Is(sit) => sit,
            SitArg::Cur => match world.ok_or(NOT_CONNECTED)?.sitting() {
                Sitting::Object(object) => SitState::Object(object),
                Sitting::Ground => SitState::Ground,
                Sitting::Standing => SitState::Stand,
                Sitting::Unnamed => {
                    return Err("the simulator has not said what the avatar sits on; \
                                give the object's UUID instead of `cur`"
                        .to_string())
                }
            },
            SitArg::Cfg => self.config.sit,
            SitArg::Last => self.last().sit,
            SitArg::LastCmd => last_sit.unwrap_or_else(|| self.last().sit),
        };
        if let (Some(location), SitState::Object(object)) = (&location, sit) {
            if let Some(region) = self
                .region_of(object, world)
                .filter(|region| !same_region(region, &location.region))
            {
                return Err(format!(
                    "the object {object} is in region \"{region}\", but the location is in \"{}\"",
                    location.region
                ));
            }
        }
        Ok(Placement { location, sit })
    }

    /// Makes `placement` the held one and what `last` stands for.
    pub fn commit(&mut self, placement: Placement) {
        self.held = placement.clone();
        self.last = Some(placement);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(region: &str, x: f32, y: f32, z: f32) -> Location {
        Location {
            region: region.to_string(),
            position: [x, y, z],
        }
    }

    fn object(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    struct Fake {
        here: Location,
        sitting: Sitting,
        known: Vec<Uuid>,
    }

    impl World for Fake {
        fn here(&self) -> Location {
            self.here.clone()
        }
        fn sitting(&self) -> Sitting {
            self.sitting
        }
        fn region_of(&self, object: Uuid) -> Option<String> {
            self.known.contains(&object).then(|| self.here.region.clone())
        }
    }

    fn standing_in(region: &str) -> Fake {
        Fake {
            here: at(region, 60.0, 70.0, 22.0),
            sitting: Sitting::Standing,
            known: Vec::new(),
        }
    }

    /// A book for a config with `region`, `position` and an object as `sit`.
    fn full() -> Book {
        Book::new(Placement {
            location: Some(at("Target", 128.0, 128.0, 25.0)),
            sit: SitState::Object(object(1)),
        })
    }

    /// A book for a config with only the account.
    fn bare() -> Book {
        Book::new(Placement {
            location: None,
            sit: SitState::Stand,
        })
    }

    fn resolve(book: &Book, location: &str, sit: &str, world: Option<&dyn World>) -> Result<Placement, String> {
        book.resolve(&parse_location(location)?, parse_sit_state(sit)?, world, None, None)
    }

    // ------------------------------------------------------------ parsing

    #[test]
    fn a_location_is_a_region_and_three_numbers() {
        let parsed = parse_location("Blue Lagoon/128/128.5/25").unwrap();
        assert_eq!(parsed, LocationArg::At(at("Blue Lagoon", 128.0, 128.5, 25.0)));
        let LocationArg::At(location) = parsed else { unreachable!() };
        assert_eq!(location.to_string(), "Blue Lagoon/128/128.5/25");
        assert_eq!(
            parse_location(" Target / 1 / 2 / 3 ").unwrap(),
            LocationArg::At(at("Target", 1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn coordinates_are_printed_to_the_centimetre() {
        assert_eq!(at("Target", 128.499, 0.0, 25.126).to_string(), "Target/128.5/0/25.13");
        assert_eq!(at("Target", 100.0, 10.5, 0.004).to_string(), "Target/100/10.5/0");
    }

    #[test]
    fn malformed_locations_are_refused_with_the_form() {
        for bad in [
            "Target/128/128",
            "Target/north/128/25",
            "Target/128/128/-1",
            "Target/128/inf/25",
            "Target/128/nan/25",
            "/128/128/25",
            "  /128/128/25",
            "A/B/128/128/25",
            "Target",
            "",
        ] {
            let reason = parse_location(bad).unwrap_err();
            assert!(reason.contains("<region>/<x>/<y>/<z>"), "{bad}: {reason}");
            assert!(reason.contains(&format!("\"{bad}\"")) || bad.trim() != bad, "{bad}: {reason}");
        }
    }

    #[test]
    fn special_words_are_read_in_any_case() {
        assert_eq!(parse_location("CUR").unwrap(), LocationArg::Cur);
        assert_eq!(parse_location("cfg").unwrap(), LocationArg::Cfg);
        assert_eq!(parse_location("Last").unwrap(), LocationArg::Last);
        assert_eq!(parse_location("Last_Cmd").unwrap(), LocationArg::LastCmd);
        assert_eq!(parse_sit_state("cur").unwrap(), SitArg::Cur);
        assert_eq!(parse_sit_state("CFG").unwrap(), SitArg::Cfg);
        assert_eq!(parse_sit_state("last").unwrap(), SitArg::Last);
        assert_eq!(parse_sit_state("LAST_CMD").unwrap(), SitArg::LastCmd);
        assert_eq!(parse_sit_state("GROUND").unwrap(), SitArg::Is(SitState::Ground));
        assert_eq!(parse_sit_state("Stand").unwrap(), SitArg::Is(SitState::Stand));
        assert_eq!(parse_sit_state("unchanged").unwrap(), SitArg::Is(SitState::Unchanged));
    }

    #[test]
    fn a_sit_state_is_a_uuid_or_a_word() {
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(
            parse_sit_state(id).unwrap(),
            SitArg::Is(SitState::Object(Uuid::parse_str(id).unwrap()))
        );
        let reason = parse_sit_state("chair").unwrap_err();
        for word in ["\"chair\"", "UUID", "ground", "stand", "unchanged", "last_cmd"] {
            assert!(reason.contains(word), "{reason}");
        }
    }

    #[test]
    fn placements_print_as_the_command_takes_them() {
        let placement = Placement {
            location: Some(at("Target", 128.0, 128.0, 25.0)),
            sit: SitState::Object(object(1)),
        };
        assert_eq!(placement.to_string(), format!("Target/128/128/25 {}", object(1)));
        for (sit, word) in [
            (SitState::Ground, "ground"),
            (SitState::Stand, "stand"),
            (SitState::Unchanged, "unchanged"),
        ] {
            assert_eq!(Placement { location: None, sit }.to_string(), format!("unknown {word}"));
            assert_eq!(parse_sit_state(word).unwrap(), SitArg::Is(sit));
        }
    }

    // ------------------------------------------------------ special words

    #[test]
    fn cur_while_seated_is_the_position_and_the_prim() {
        let world = Fake {
            here: at("Target", 131.0, 64.0, 23.0),
            sitting: Sitting::Object(object(9)),
            known: vec![],
        };
        let placement = resolve(&full(), "cur", "cur", Some(&world)).unwrap();
        assert_eq!(
            placement,
            Placement {
                location: Some(world.here.clone()),
                sit: SitState::Object(object(9))
            }
        );
    }

    #[test]
    fn cur_on_the_ground_and_standing() {
        let mut world = standing_in("Target");
        assert_eq!(resolve(&full(), "cur", "cur", Some(&world)).unwrap().sit, SitState::Stand);
        world.sitting = Sitting::Ground;
        assert_eq!(resolve(&full(), "cur", "cur", Some(&world)).unwrap().sit, SitState::Ground);
    }

    #[test]
    fn cur_needs_the_avatar_in_a_region() {
        let book = full();
        assert_eq!(resolve(&book, "cur", "stand", None).unwrap_err(), NOT_CONNECTED);
        assert_eq!(resolve(&book, "cfg", "cur", None).unwrap_err(), NOT_CONNECTED);
        // The other words and full values do not.
        assert!(resolve(&book, "cfg", "cfg", None).is_ok());
        assert!(resolve(&book, "last", "last_cmd", None).is_ok());
        assert!(resolve(&book, "Home/1/2/3", "ground", None).is_ok());
    }

    #[test]
    fn cur_refuses_a_seat_the_simulator_did_not_name() {
        let mut world = standing_in("Target");
        world.sitting = Sitting::Unnamed;
        let reason = resolve(&full(), "cur", "cur", Some(&world)).unwrap_err();
        assert!(reason.contains("UUID"), "{reason}");
        // Only the sit state needs the name.
        assert!(resolve(&full(), "cur", "unchanged", Some(&world)).is_ok());
    }

    #[test]
    fn cfg_is_the_config_placement() {
        let placement = resolve(&full(), "cfg", "cfg", None).unwrap();
        assert_eq!(
            placement,
            Placement {
                location: Some(at("Target", 128.0, 128.0, 25.0)),
                sit: SitState::Object(object(1))
            }
        );
    }

    #[test]
    fn cfg_without_a_position_is_the_first_location() {
        let mut book = bare();
        // Not known yet: accepted, and no location is held.
        assert_eq!(
            resolve(&book, "cfg", "cfg", None).unwrap(),
            Placement {
                location: None,
                sit: SitState::Stand
            }
        );
        assert!(book.first_location(at("Home", 60.0, 70.0, 22.0)));
        let placement = resolve(&book, "cfg", "cfg", None).unwrap();
        assert_eq!(
            placement,
            Placement {
                location: Some(at("Home", 60.0, 70.0, 22.0)),
                sit: SitState::Stand
            }
        );
        // Only the first report counts.
        assert!(!book.first_location(at("Target", 1.0, 2.0, 3.0)));
        assert_eq!(
            resolve(&book, "cfg", "cfg", None).unwrap().location,
            Some(at("Home", 60.0, 70.0, 22.0))
        );
    }

    #[test]
    fn a_configured_position_is_not_replaced_by_the_first_location() {
        let mut book = full();
        assert!(!book.first_location(at("Home", 60.0, 70.0, 22.0)));
        assert_eq!(book.held().location, Some(at("Target", 128.0, 128.0, 25.0)));
        assert_eq!(
            resolve(&book, "cfg", "cfg", None).unwrap().location,
            Some(at("Target", 128.0, 128.0, 25.0))
        );
    }

    #[test]
    fn last_is_the_last_persist_or_else_the_config() {
        let mut book = full();
        assert_eq!(
            resolve(&book, "last", "last", None).unwrap(),
            resolve(&book, "cfg", "cfg", None).unwrap()
        );
        let persisted = Placement {
            location: Some(at("Blue Lagoon", 10.0, 20.0, 30.0)),
            sit: SitState::Ground,
        };
        book.commit(persisted.clone());
        assert_eq!(book.held(), &persisted);
        assert_eq!(resolve(&book, "last", "last", None).unwrap(), persisted);
        // The config values are still there to go back to.
        assert_eq!(resolve(&book, "cfg", "cfg", None).unwrap().sit, SitState::Object(object(1)));
        // Each argument is taken by itself.
        let mixed = resolve(&book, "cfg", "last", None).unwrap();
        assert_eq!(
            mixed,
            Placement {
                location: Some(at("Target", 128.0, 128.0, 25.0)),
                sit: SitState::Ground
            }
        );
    }

    #[test]
    fn last_cmd_is_the_last_tp_and_sit_or_else_last() {
        let mut book = full();
        book.commit(Placement {
            location: Some(at("Blue Lagoon", 10.0, 20.0, 30.0)),
            sit: SitState::Unchanged,
        });
        let (location, sit) = (LocationArg::LastCmd, SitArg::LastCmd);
        // No tp and no sit command yet: as `last`.
        let placement = book.resolve(&location, sit, None, None, None).unwrap();
        assert_eq!(placement, resolve(&book, "last", "last", None).unwrap());
        let tp = at("Home", 100.0, 120.0, 30.0);
        let placement = book.resolve(&location, sit, None, Some(&tp), None).unwrap();
        assert_eq!(
            placement,
            Placement {
                location: Some(tp.clone()),
                sit: SitState::Unchanged
            }
        );
        let placement = book.resolve(&location, sit, None, None, Some(SitState::Object(object(7)))).unwrap();
        assert_eq!(placement.location, Some(at("Blue Lagoon", 10.0, 20.0, 30.0)));
        assert_eq!(placement.sit, SitState::Object(object(7)));
        let placement = book.resolve(&location, sit, None, Some(&tp), Some(SitState::Ground)).unwrap();
        assert_eq!(
            placement,
            Placement {
                location: Some(tp),
                sit: SitState::Ground
            }
        );
    }

    #[test]
    fn a_failed_resolve_changes_nothing() {
        let mut book = full();
        book.commit(Placement {
            location: Some(at("Home", 1.0, 2.0, 3.0)),
            sit: SitState::Ground,
        });
        let before = format!("{book:?}");
        assert!(resolve(&book, "cur", "cur", None).is_err());
        assert!(resolve(&book, "Target/1/2", "cur", None).is_err());
        assert!(resolve(&book, "cfg", "chair", None).is_err());
        assert_eq!(format!("{book:?}"), before);
    }

    #[test]
    fn a_location_that_waited_takes_the_first_location() {
        let mut book = bare();
        let placement = resolve(&book, "cfg", "cfg", None).unwrap();
        book.commit(placement);
        assert_eq!(book.held().location, None);
        assert!(book.first_location(at("Home", 60.0, 70.0, 22.0)));
        assert_eq!(book.held().location, Some(at("Home", 60.0, 70.0, 22.0)));
        assert_eq!(
            resolve(&book, "last", "last", None).unwrap().location,
            Some(at("Home", 60.0, 70.0, 22.0))
        );
    }

    #[test]
    fn a_persisted_location_does_not_wait_for_the_first() {
        let mut book = bare();
        book.commit(Placement {
            location: Some(at("Target", 1.0, 2.0, 3.0)),
            sit: SitState::Stand,
        });
        assert!(!book.first_location(at("Home", 60.0, 70.0, 22.0)));
        assert_eq!(book.held().location, Some(at("Target", 1.0, 2.0, 3.0)));
        assert_eq!(
            resolve(&book, "last", "last", None).unwrap().location,
            Some(at("Target", 1.0, 2.0, 3.0))
        );
    }

    // --------------------------------------------------- the region check

    #[test]
    fn seat_here_and_location_elsewhere_is_refused() {
        let mut book = full();
        book.seated(object(9), "Target");
        let world = Fake {
            here: at("Target", 131.0, 64.0, 23.0),
            sitting: Sitting::Object(object(9)),
            known: vec![],
        };
        let reason = resolve(&book, "Home/128/128/25", "cur", Some(&world)).unwrap_err();
        assert!(reason.contains("\"Home\"") && reason.contains("\"Target\""), "{reason}");
        assert_eq!(book.held(), full().held());
    }

    #[test]
    fn location_here_and_seat_elsewhere_is_refused() {
        let mut book = full();
        book.seated(object(9), "Target");
        let world = standing_in("Home");
        let reason = resolve(&book, "cur", &object(9).to_string(), Some(&world)).unwrap_err();
        assert!(reason.contains("\"Home\"") && reason.contains("\"Target\""), "{reason}");
        // Also without a session, and through the special words.
        book.commit(Placement {
            location: Some(at("Target", 1.0, 2.0, 3.0)),
            sit: SitState::Object(object(9)),
        });
        assert!(resolve(&book, "Home/1/2/3", "last", None).is_err());
        assert!(resolve(&book, "last", "last", None).is_ok());
    }

    #[test]
    fn an_object_the_bot_has_not_met_passes() {
        let book = full();
        let world = standing_in("Target");
        assert!(resolve(&book, "Home/128/128/25", &object(5).to_string(), Some(&world)).is_ok());
        assert!(resolve(&book, "Home/128/128/25", &object(5).to_string(), None).is_ok());
    }

    #[test]
    fn an_object_learned_in_the_current_region_is_known_to_be_there() {
        let book = full();
        let mut world = standing_in("Target");
        world.known.push(object(5));
        assert!(resolve(&book, "Home/128/128/25", &object(5).to_string(), Some(&world)).is_err());
        assert!(resolve(&book, "cur", &object(5).to_string(), Some(&world)).is_ok());
    }

    #[test]
    fn regions_are_compared_without_case_or_padding() {
        let mut book = full();
        book.seated(object(9), "Target");
        assert!(resolve(&book, "target/1/2/3", &object(9).to_string(), None).is_ok());
        assert!(resolve(&book, " TARGET /1/2/3", &object(9).to_string(), None).is_ok());
    }

    #[test]
    fn the_newest_region_of_an_object_counts() {
        let mut book = full();
        book.seated(object(9), "Target");
        book.seated(object(9), "Home");
        assert!(resolve(&book, "Home/1/2/3", &object(9).to_string(), None).is_ok());
        assert!(resolve(&book, "Target/1/2/3", &object(9).to_string(), None).is_err());
    }

    #[test]
    fn sixty_four_seats_are_remembered() {
        let mut book = full();
        for n in 100..=164 {
            book.seated(object(n), "Target");
        }
        // The oldest of the 65 is forgotten, the next one is not.
        assert!(resolve(&book, "Home/1/2/3", &object(100).to_string(), None).is_ok());
        assert!(resolve(&book, "Home/1/2/3", &object(101).to_string(), None).is_err());
        assert!(resolve(&book, "Home/1/2/3", &object(164).to_string(), None).is_err());
    }
}
