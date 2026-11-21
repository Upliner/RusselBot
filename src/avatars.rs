//! The `scan avatars` command's own part: where the avatars in the region
//! are, how far, and the listing. No session and no clock: what the
//! simulator has told comes in as `Seen`s.

use crate::escape::escape;
use crate::names::Names;
use crate::udp::messages::Vector3;
use std::cmp::Ordering;
use std::collections::HashMap;
use uuid::Uuid;

/// An avatar in the region other than the bot's own, as the simulator has
/// told the bot of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub id: Uuid,
    /// East and north in the region, in metres.
    pub position: [f32; 2],
    /// Height in the region, in metres; `None` when the simulator does not
    /// tell it.
    pub height: Option<f32>,
    /// An RLV restriction hides who it is.
    pub hidden: bool,
}

impl Seen {
    /// How far it is from `from`, in metres; `None` without its height.
    pub fn distance(&self, from: Vector3) -> Option<f32> {
        let [x, y] = self.position;
        let z = self.height?;
        Some([x - from[0], y - from[1], z - from[2]].iter().map(|d| d * d).sum::<f32>().sqrt())
    }

    /// Whether it is known to be no more than `distance` metres from `from`.
    pub fn within(&self, from: Vector3, distance: f32) -> bool {
        self.distance(from).is_some_and(|d| d <= distance)
    }
}

/// The order of the listing: nearest first, those without a distance
/// last, then by UUID.
fn order(a: &(Option<f32>, &Seen), b: &(Option<f32>, &Seen)) -> Ordering {
    let by_distance = match (a.0, b.0) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    by_distance.then(a.1.id.cmp(&b.1.id))
}

/// The listing of `seen` as measured from `from`: one line each of the
/// UUID, the distance, the position in whole metres, the name and the
/// display name, separated by tabs, with `?` for what is not known or is
/// hidden. `names` are the avatars' names as far as the name service gave
/// them; `minimap` hides every position. `None` when there is nothing to
/// list.
pub fn listing(seen: &[Seen], from: Vector3, names: &HashMap<Uuid, Option<Names>>, minimap: bool) -> Option<String> {
    let mut measured: Vec<(Option<f32>, &Seen)> = seen.iter().map(|seen| (seen.distance(from), seen)).collect();
    measured.sort_by(order);
    let lines: Vec<String> = measured
        .iter()
        .map(|(distance, seen)| {
            let unknown = || "?".to_string();
            let distance = distance.map_or_else(unknown, |d| format!("{d:.1}"));
            let position = match minimap {
                true => unknown(),
                false => {
                    let [x, y] = seen.position.map(|v| format!("{}", v.round() as i64));
                    let z = seen.height.map_or_else(unknown, |z| format!("{}", z.round() as i64));
                    format!("{x},{y},{z}")
                }
            };
            let named = names.get(&seen.id).and_then(Option::as_ref).filter(|_| !seen.hidden);
            let (id, name, display) = match (seen.hidden, named) {
                (true, _) => (unknown(), unknown(), unknown()),
                (false, None) => (seen.id.to_string(), unknown(), unknown()),
                (false, Some(names)) => (seen.id.to_string(), escape(&names.legacy), escape(&names.display)),
            };
            format!("{id}\t{distance}\t{position}\t{name}\t{display}")
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: Vector3 = [100.0, 100.0, 25.0];

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn at(n: u8, x: f32, y: f32, z: Option<f32>) -> Seen {
        Seen {
            id: id(n),
            position: [x, y],
            height: z,
            hidden: false,
        }
    }

    /// An avatar `metres` east of the bot's.
    fn east(n: u8, metres: f32) -> Seen {
        at(n, 100.0 + metres, 100.0, Some(25.0))
    }

    fn names() -> HashMap<Uuid, Option<Names>> {
        let named = |legacy: &str, display: &str| {
            Some(Names {
                legacy: legacy.to_string(),
                display: display.to_string(),
            })
        };
        HashMap::from([(id(1), named("Jane Doe", "JD")), (id(2), named("John Roe", "Johnny"))])
    }

    fn first_fields(text: &str) -> Vec<String> {
        text.lines().map(|line| line.split('\t').nth(1).unwrap().to_string()).collect()
    }

    #[test]
    fn where_an_avatar_is() {
        // Described: as the simulator gave it.
        let described = at(1, 103.24, 100.0, Some(25.0));
        assert_eq!(format!("{:.1}", described.distance(HERE).unwrap()), "3.2");
        // Known by the list only, in its whole metres and steps of 4 m.
        let listed = at(2, 200.0, 40.0, Some(24.0));
        let line = listing(&[listed], HERE, &HashMap::new(), false).unwrap();
        assert_eq!(line, format!("{}\t116.6\t200,40,24\t?\t?", id(2)));
        // Above the bot: three dimensions.
        let above = at(3, 100.0, 100.0, Some(55.0));
        assert_eq!(above.distance(HERE), Some(30.0));
    }

    #[test]
    fn the_distance_filter() {
        let near = [east(1, 8.0), east(2, 15.0), east(3, 60.0)];
        let within = |metres: f32| -> Vec<Uuid> { near.iter().filter(|s| s.within(HERE, metres)).map(|s| s.id).collect() };
        assert_eq!(within(20.0), [id(1), id(2)]);
        assert_eq!(within(10.0), [id(1)]);
        assert_eq!(within(100.0), [id(1), id(2), id(3)]);
        assert_eq!(within(12.5), [id(1)]);
        // Above the bot by 30 m.
        let above = at(4, 100.0, 100.0, Some(55.0));
        assert!(!above.within(HERE, 20.0) && above.within(HERE, 40.0));
        // An unknown distance is within nothing.
        let unknown = at(5, 128.0, 64.0, None);
        assert_eq!(unknown.distance(HERE), None);
        assert!(!unknown.within(HERE, 1024.0));
    }

    #[test]
    fn a_line_has_five_fields() {
        let jane = at(1, 103.24, 100.0, Some(25.0));
        assert_eq!(
            listing(&[jane], HERE, &names(), false).unwrap(),
            format!("{}\t3.2\t103,100,25\tJane Doe\tJD", id(1))
        );
    }

    #[test]
    fn what_is_not_known_is_a_question_mark() {
        let high = at(1, 128.0, 64.0, None);
        assert_eq!(
            listing(&[high], HERE, &names(), false).unwrap(),
            format!("{}\t?\t128,64,?\tJane Doe\tJD", id(1))
        );
        // Not named, or named as nobody.
        let jane = east(1, 2.0);
        for names in [HashMap::new(), HashMap::from([(id(1), None)])] {
            let line = listing(std::slice::from_ref(&jane), HERE, &names, false).unwrap();
            assert_eq!(line, format!("{}\t2.0\t102,100,25\t?\t?", id(1)));
        }
    }

    #[test]
    fn names_stay_in_their_field() {
        let odd = HashMap::from([(
            id(1),
            Some(Names {
                legacy: "Jane Doe".to_string(),
                display: "Tab\there\\n".to_string(),
            }),
        )]);
        let line = listing(&[east(1, 1.0)], HERE, &odd, false).unwrap();
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[4], "Tab\\there\\\\n");
    }

    #[test]
    fn nearest_first_unknown_last_then_uuid() {
        let seen = [east(3, 60.0), east(1, 8.0), at(0, 1.0, 1.0, None), east(2, 15.0), east(4, 8.0)];
        let text = listing(&seen, HERE, &names(), false).unwrap();
        assert_eq!(first_fields(&text), ["8.0", "8.0", "15.0", "60.0", "?"]);
        let ids: Vec<&str> = text.lines().map(|line| line.split('\t').next().unwrap()).collect();
        assert_eq!(ids[..2], [id(1).to_string(), id(4).to_string()]);
        assert_eq!(listing(&[], HERE, &names(), false), None);
    }

    #[test]
    fn hidden_avatars_keep_distance_position_and_order() {
        let hidden = |seen: Seen| Seen { hidden: true, ..seen };
        let jane = hidden(at(1, 103.24, 100.0, Some(25.0)));
        assert_eq!(
            listing(std::slice::from_ref(&jane), HERE, &names(), false).unwrap(),
            "?\t3.2\t103,100,25\t?\t?"
        );
        let seen = [east(3, 60.0), hidden(east(2, 15.0)), hidden(east(1, 8.0))];
        let text = listing(&seen, HERE, &names(), false).unwrap();
        assert_eq!(first_fields(&text), ["8.0", "15.0", "60.0"]);
        assert!(text.lines().last().unwrap().starts_with(&id(3).to_string()));
    }

    #[test]
    fn the_minimap_restriction_hides_positions() {
        let jane = at(1, 103.24, 100.0, Some(25.0));
        assert_eq!(
            listing(std::slice::from_ref(&jane), HERE, &names(), true).unwrap(),
            format!("{}\t3.2\t?\tJane Doe\tJD", id(1))
        );
        let hidden = Seen {
            hidden: true,
            ..east(2, 8.0)
        };
        assert_eq!(listing(&[hidden], HERE, &names(), true).unwrap(), "?\t8.0\t?\t?\t?");
    }
}
